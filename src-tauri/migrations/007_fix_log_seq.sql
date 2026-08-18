-- Add a stable sequence cursor for existing and future request logs.
ALTER TABLE request_logs ADD COLUMN seq INTEGER;

UPDATE request_logs SET seq = rowid WHERE seq = 0 OR seq IS NULL;
