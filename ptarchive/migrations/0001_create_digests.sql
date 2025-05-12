-- Enable write-ahead logging for better concurrency
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

CREATE TABLE IF NOT EXISTS digests (
    digest          TEXT    PRIMARY KEY,
    original_digest TEXT    NOT NULL,
    filename        TEXT    NOT NULL,
    absolute_path   TEXT    NOT NULL,
    length          INT     NOT NULL,
    unique_id       TEXT    NULL,
    session         TEXT    NOT NULL,
    hdd             TEXT    NOT NULL,
    repository      TEXT    NOT NULL,
    registry        TEXT    NOT NULL,
    tag             TEXT    NOT NULL
);

