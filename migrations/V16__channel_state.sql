CREATE TABLE IF NOT EXISTS channel_state (
    channel_name TEXT NOT NULL,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel_name, path)
);

CREATE INDEX idx_channel_state_prefix ON channel_state (channel_name, path text_pattern_ops);
