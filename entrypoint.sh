#!/bin/bash
set -e

# Delete old embedded DB so onboarding re-runs with correct env vars
rm -f /root/.ironclaw/ironclaw.db

# Run ironclaw
exec ironclaw "$@"
