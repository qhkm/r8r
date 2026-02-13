#!/bin/sh
set -e

# Fix permissions on data directory if running as root
if [ "$(id -u)" = '0' ]; then
    # Ensure data directory exists and has correct ownership
    mkdir -p /data
    chown -R r8r:r8r /data

    # Drop to r8r user and execute command
    exec gosu r8r "$@"
fi

exec "$@"
