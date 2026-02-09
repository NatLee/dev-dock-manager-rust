#!/bin/sh
# 確保 SQLite 資料目錄存在且可寫，避免 code 14 (unable to open database file)
# sqlx 依 URL 解析可能用 /app/data、app/data（相對於 /app）、或 /data，一併建立
set -e
mkdir -p /app/data /app/app/data /data
chmod 777 /app/data /app/app/data /data
exec "$@"
