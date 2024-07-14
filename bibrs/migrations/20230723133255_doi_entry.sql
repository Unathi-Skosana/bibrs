CREATE TABLE IF NOT EXISTS doi_entries (
    cite_key        TEXT PRIMARY KEY,
    bib_type        TEXT NOT NULL,
    doi             TEXT NOT NULL,
    url             TEXT NOT NULL,
    author          TEXT NOT NULL,
    title           TEXT NOT NULL,
    journal         TEXT NOT NULL,
    publisher       TEXT NOT NULL,
    volume          INT NOT NULL,
    number          INT NOT NULL,
    month           TEXT NOT NULL,
    year            INT NOT NULL
)
