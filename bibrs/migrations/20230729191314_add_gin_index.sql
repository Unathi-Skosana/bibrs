-- Add migration script here
ALTER TABLE doi_entries ADD search tsvector GENERATED ALWAYS AS
    (
        to_tsvector('simple',title) || ' ' ||
        to_tsvector('simple',author) || ' ' ||
        to_tsvector('simple', journal) || ' ' ||
        to_tsvector('simple', publisher)
) STORED;


CREATE INDEX idx_search ON doi_entries USING GIN(search);
