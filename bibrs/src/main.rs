use clap::{arg, Command};
use nom_bibtex::*;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::types::JsonValue;
use sqlx::Row;
use std::io::Write;
use std::{env::var, fs::File, io::Read, process::Command as CMD, process::Stdio, str};
use tempfile::NamedTempFile;
use tracing::info;
use tracing_subscriber;

const DOI_URL: &str = "https://doi.org/";
const DATABASE_URL: &str = "postgres://postgres:password@localhost/bibrs";

#[derive(Deserialize, Serialize)]
pub struct DOIEntry {
    pub cite_key: String,
    pub bib_type: String,
    pub doi: String,
    pub url: String,
    pub author: String,
    pub title: String,
    pub journal: String,
    pub publisher: String,
    pub volume: i64,
    pub number: i64,
    pub month: String,
    pub year: i64,
}

impl DOIEntry {
    fn new(raw_biblatex: &str) -> Self {
        let bibtex = Bibtex::parse(&raw_biblatex).unwrap();
        let biblio = &bibtex.bibliographies()[0];

        let bib_type = biblio.entry_type();
        let cite_key = biblio.citation_key();
        let tags = biblio.tags();

        // TODO: Be safe playa
        Self {
            cite_key: String::from(cite_key),
            bib_type: String::from(bib_type),
            doi: String::from(&tags["doi"]),
            url: String::from(&tags["url"]),
            author: String::from(&tags["author"]),
            title: String::from(&tags["title"]),
            journal: String::from(&tags["journal"]),
            publisher: String::from(&tags["publisher"]),
            volume: String::from(&tags["volume"]).parse::<i64>().unwrap(),
            number: String::from(&tags["number"]).parse::<i64>().unwrap(),
            month: String::from(&tags["month"]),
            year: String::from(&tags["year"]).parse::<i64>().unwrap(),
        }
    }
}

fn cli() -> Command {
    Command::new("bibrs")
        .version("1.0")
        .author("UKS <ukskosana@gmail.com>")
        .about("Bibliography manager")
        .subcommand_required(true)
        .subcommand(
            Command::new("add")
                .about("Adds entry to the bibliography")
                .args([arg!(<entry> "Entry to add")])
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("delete")
                .about("Deletes entry to the bibliography")
                .args([
                    arg!(-k --key <key> "Entry to delete"),
                    arg!(-i --interactive "List entries to delete interactively"),
                ])
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("edit")
                .about("Edit entries in the bibliography")
                .args([
                    arg!(-k --key <key> "Citation key of entry to edit"),
                    arg!(-i --interactive "Select entries to edit interactively"),
                ])
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("list")
                .about("Lists entries in the bibliography")
                .args([
                    arg!(-i --interactive "List entries interactively"),
                    arg!(-t --tag <tag> "List entries that match tag"),
                    arg!(-q --query <query> "List entries that match query"),
                ])
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("export")
                .about("Exports bibliography to a file")
                .args([arg!(<filename> "Filename to export to a biblatex file")])
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("import")
                .about("Imports bibliography from a file")
                .args([arg!(<filename> "Filename from a biblatex file")])
                .arg_required_else_help(true),
        )
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Initialize HTTP client with headers
    let mut headers = header::HeaderMap::new();
    headers.insert("Accept", "application/x-bibtex".parse().unwrap());
    headers.insert("User-Agent", "bibrs/1.0".parse().unwrap());
    let client = reqwest::Client::new();

    // Initialize database connection pool
    let pool = PgPoolOptions::new()
        .max_connections(100)
        .connect(DATABASE_URL)
        .await?;

    // Run database migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    // Parse command line arguments
    let matches = cli().get_matches();

    // Match and handle subcommands
    match matches.subcommand() {
        Some(("add", sub_matches)) => {
            let entry = sub_matches.get_one::<String>("entry").expect("required");

            let url = format!("{DOI_URL}/{entry}");

            let response = client.get(url).headers(headers).send().await?;

            let raw_bibtex = response.text().await?;

            let record = DOIEntry::new(&raw_bibtex);

            // TODO: Clashing key resolution
            // idea1 :: a cite_key that already exists,
            // we can modify the cite_key of the
            // new entry, i.e. add alphabet/number to
            // the end of it.
            // idea2 :: open entry as json, and let user resolve
            // the clash themselves.
            // I think idea 2 is easier

            let _ = add_entry(&pool, &record).await?;
        }
        Some(("delete", sub_matches)) => {
            let key = sub_matches.get_one::<String>("key").expect("required");

            let _ = delete_entry(&pool, &key).await?;
        }
        Some(("edit", sub_matches)) => {
            // TODO:: idea 1 :: When we edit an entry
            // Load the row to temporary file
            // Open the file with the default editor (similar to commit message)
            // Once the edit is finished
            // Read from row temporary file
            // Insert back into db

            if let Some(key) = sub_matches.get_one::<String>("key") {
                edit_entry(&pool, &key).await?;
            } else {
                let entries: Vec<PgRow> = list_entries(&pool).await?;
                let key = run_fzf_pipeline(entries)?;

                if !key.is_empty() {
                    info!("Pick a key {}", key);
                    edit_entry(&pool, &key).await?;
                } else {
                    info!("Did not pick any key")
                }
            }
        }
        Some(("list", sub_matches)) => {
            if let Some(query) = sub_matches.get_one::<String>("query") { // handle query searches
                let entries: Vec<PgRow> = list_query_matches(&pool, &query).await?;
                entries
                    .iter()
                    .for_each(|t| info!("{}", t.get::<String, _>("title")))
            } else if let Some(tag) = sub_matches.get_one::<String>("tag") { // handle tag searches
                let entries: Vec<PgRow> = list_query_matches(&pool, &tag).await?;
                entries
                    .iter()
                    .for_each(|t| info!("{}", t.get::<String, _>("title")));
            } else { // interactive search
                let entries: Vec<PgRow> = list_entries(&pool).await?;
                let key = run_fzf_pipeline(entries)?;

                if !key.is_empty() {
                    info!("Pick a key {}", key);
                } else {
                    info!("Did not pick any key")
                }
            }
        }
        _ => unreachable!(), // If all subcommands are defined above, anything else is unreachable!()
    }

    Ok(())
}

// Function to add a new entry to the database
async fn add_entry(pool: &PgPool, doi_entry: &DOIEntry) -> anyhow::Result<PgRow> {
    let rec = sqlx::query(
        "
        insert into doi_entries
        (cite_key, bib_type, doi, url, author, title, journal, publisher, volume, number, month, year)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        returning *
        "
    )
    .bind(&doi_entry.cite_key)
    .bind(&doi_entry.bib_type)
    .bind(&doi_entry.doi)
    .bind(&doi_entry.url)
    .bind(&doi_entry.author)
    .bind(&doi_entry.title)
    .bind(&doi_entry.journal)
    .bind(&doi_entry.publisher)
    .bind(&doi_entry.volume)
    .bind(&doi_entry.number)
    .bind(&doi_entry.month)
    .bind(&doi_entry.year)
    .fetch_one(pool)
    .await?;

    Ok(rec)
}

// Function to update an existing entry in the database
async fn update_entry(pool: &PgPool, key: &str, doi_entry: &DOIEntry) -> anyhow::Result<PgRow> {
    let rec = sqlx::query(
        "
        update doi_entries
        set cite_key = $1,
            bib_type = $2,
            doi = $3,
            url = $4,
            author = $5,
            title = $6,
            journal = $7,
            publisher = $8,
            volume = $9,
            number = $10,
            month = $11,
            year = $12
        where cite_key = $13
        returning *
        ",
    )
    .bind(&doi_entry.cite_key)
    .bind(&doi_entry.bib_type)
    .bind(&doi_entry.doi)
    .bind(&doi_entry.url)
    .bind(&doi_entry.author)
    .bind(&doi_entry.title)
    .bind(&doi_entry.journal)
    .bind(&doi_entry.publisher)
    .bind(&doi_entry.volume)
    .bind(&doi_entry.number)
    .bind(&doi_entry.month)
    .bind(&doi_entry.year)
    .bind(&key)
    .fetch_one(pool)
    .await?;

    Ok(rec)
}

// Function to delete an entry from the database
async fn delete_entry(pool: &PgPool, key: &str) -> anyhow::Result<PgRow> {
    let rec = sqlx::query("delete from doi_entries where cite_key = $1 returning *")
        .bind(&key)
        .fetch_one(pool)
        .await?;

    Ok(rec)
}

// Function to list all entries in the database
async fn list_entries(pool: &PgPool) -> anyhow::Result<Vec<PgRow>> {
    let recs = sqlx::query("select * from doi_entries")
        .fetch_all(pool)
        .await?;

    Ok(recs)
}

// Function to list entries that match a query
async fn list_query_matches(pool: &PgPool, query: &str) -> anyhow::Result<Vec<PgRow>> {
    // https://xata.io/blog/postgres-full-text-search-engine
    let recs = sqlx::query(
        "
        select * from doi_entries
        where search @@ websearch_to_tsquery('simple', $1)
        ",
    )
    .bind(&query)
    .fetch_all(pool)
    .await?;

    Ok(recs)
}

// Function to convert an entry to JSON format
async fn entry_to_json(pool: &PgPool, cite_key: &str) -> anyhow::Result<JsonValue> {
    //https://www.commandprompt.com/education/postgresql-json_agg-function-by-practical-examples/
    //https://www.reddit.com/r/rust/comments/vdggo6/sqlx_postgres_result_to_json/
    let rec = sqlx::query(
        "
            with doi as
            (
                select cite_key, bib_type, doi, url, author, title,
                journal, publisher, volume,
                number, month, year from doi_entries
                where cite_key = $1
            ) select row_to_json(doi.*, true) from doi
        ",
    )
    .bind(cite_key)
    .fetch_one(pool)
    .await?;

    let json_rec = rec.get::<Value, _>(0);

    Ok(json_rec)
}

// Function to edit an entry in the database
async fn edit_entry(pool: &PgPool, key: &str) -> anyhow::Result<()> {
    let json_entry = entry_to_json(pool, key).await?;

    // Create a temporary file inside of the directory returned by `std::env::temp_dir()`.
    let mut temp_file = NamedTempFile::new()?;

    let old_entry_str = serde_json::to_string_pretty(&json_entry).unwrap();

    let _ = write!(temp_file, "{}", &old_entry_str);

    let editor = var("EDITOR").unwrap();

    CMD::new(editor)
        .arg(&temp_file.path())
        .status()
        .expect("Something went wrong");

    let mut editable = String::new();
    let _ = File::open(&temp_file.path())
        .expect("Could not open file")
        .read_to_string(&mut editable);

    if editable.trim() != old_entry_str.trim() {
        info!("{}", "edited");
        let new_entry: DOIEntry = serde_json::from_str(&editable)?;
        let _ = update_entry(pool, key, &new_entry).await?;
    }

    temp_file.close()?;
    Ok(())
}

// Function to run the fzf pipeline
fn run_fzf_pipeline(entries: Vec<PgRow>) -> anyhow::Result<String> {
    let fzf = entries
        .iter()
        .map(|s| s.get::<String, _>("cite_key"))
        .reduce(|a: String, b: String| a + "\n" + &b)
        .unwrap();

    let echo_child = CMD::new("echo")
        .arg(fzf)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let echo_out = echo_child.stdout.expect("Failed to open echo stdout");

    let fzf_child = CMD::new("fzf")
        .stdin(Stdio::from(echo_out))
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start sed process");

    let output = fzf_child.wait_with_output().unwrap();
    let key = str::from_utf8(&output.stdout).unwrap().trim().to_string();

    Ok(key)
}
