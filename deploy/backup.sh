#!/usr/bin/env bash
# Hot-backup the GraveTender report database and retain two weeks.
set -euo pipefail

DB="${GRAVE_TENDER_DB:-/var/lib/gravetender/reports.db}"
DEST="${GRAVE_TENDER_BACKUP_DIR:-/var/backups/gravetender}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"

install -d -m 0750 "$DEST"
sqlite3 "$DB" ".backup '$DEST/reports-$STAMP.db'"
find "$DEST" -type f -name 'reports-*.db' -mtime +14 -delete

# Optional off-instance copy:
# aws s3 cp "$DEST/reports-$STAMP.db" "s3://your-bucket/gravetender/" --only-show-errors
