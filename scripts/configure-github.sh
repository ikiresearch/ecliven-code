#!/usr/bin/env bash
# Applies the recommended GitHub settings for this repository.
# Requires the GitHub CLI authenticated as a repository admin: gh auth login
# Usage: scripts/configure-github.sh [owner/repo]
set -euo pipefail

REPO="${1:-ikiresearch/ecliven-code}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

step() { printf '\n==> %s\n' "$1"; }

step "Repository metadata and merge settings"
gh api -X PATCH "repos/$REPO" \
  -f homepage="https://ecliven.com/code" \
  -F has_wiki=false \
  -F allow_squash_merge=true \
  -F allow_merge_commit=false \
  -F allow_rebase_merge=false \
  -F allow_auto_merge=true \
  -F delete_branch_on_merge=true \
  -f squash_merge_commit_title=PR_TITLE \
  -f squash_merge_commit_message=PR_BODY >/dev/null

gh api -X PUT "repos/$REPO/topics" \
  -f "names[]=rust" -f "names[]=ai" -f "names[]=coding-agent" \
  -f "names[]=cli" -f "names[]=tui" -f "names[]=llm" >/dev/null

step "Secret scanning and push protection"
gh api -X PATCH "repos/$REPO" --input - >/dev/null <<'JSON'
{
  "security_and_analysis": {
    "secret_scanning": { "status": "enabled" },
    "secret_scanning_push_protection": { "status": "enabled" }
  }
}
JSON

step "Private vulnerability reporting"
gh api -X PUT "repos/$REPO/private-vulnerability-reporting" >/dev/null

step "Dependabot alerts and security updates"
gh api -X PUT "repos/$REPO/vulnerability-alerts" >/dev/null
gh api -X PUT "repos/$REPO/automated-security-fixes" >/dev/null

step "Code scanning (CodeQL default setup)"
gh api -X PATCH "repos/$REPO/code-scanning/default-setup" -f state=configured >/dev/null \
  || echo "   CodeQL default setup could not be enabled; enable it in Settings > Code security."

step "Actions: read-only token by default"
gh api -X PUT "repos/$REPO/actions/permissions/workflow" \
  -f default_workflow_permissions=read \
  -F can_approve_pull_request_reviews=false >/dev/null

step "Rulesets"
for file in "$ROOT"/.github/rulesets/*.json; do
  name="$(jq -r .name "$file")"
  id="$(gh api "repos/$REPO/rulesets" --jq ".[] | select(.name == \"$name\") | .id")"
  if [ -n "$id" ]; then
    gh api -X PUT "repos/$REPO/rulesets/$id" --input "$file" >/dev/null
    echo "   updated: $name"
  else
    gh api -X POST "repos/$REPO/rulesets" --input "$file" >/dev/null
    echo "   created: $name"
  fi
done

step "Done"
