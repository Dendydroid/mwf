-- Add up migration script here
CREATE TABLE IF NOT EXISTS test_table
(
    test       VARCHAR(100),
    created_at BIGINT
);