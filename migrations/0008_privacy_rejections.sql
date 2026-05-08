-- ADR-0009: aggregate rejection telemetry without secret content or hashes.
CREATE TABLE IF NOT EXISTS privacy_rejections (
  reason TEXT PRIMARY KEY,
  count  INTEGER NOT NULL DEFAULT 0
);
