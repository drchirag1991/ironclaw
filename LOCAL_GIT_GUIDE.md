# Local Git Guide for Safe Fork & Public Repo Workflow

This guide explains the safest local git workflow when working with a fork of a public repository like `nearai/ironclaw`.

It is meant as a local reference for AI platforms, agents, and skills that need to operate against a forked public repo while preserving safe history and secret-handling practices.

## 1. Set up remotes correctly

Use your fork for development and the public repository as an upstream read-only source.

```powershell
cd E:\projects\ironclaw\ironclaw

git remote add fork https://github.com/drchirag1991/ironclaw.git
git remote add upstream https://github.com/nearai/ironclaw.git
```

Verify remotes:

```powershell
git remote -v
```

## 2. Sync safely from upstream

Fetch the latest state from the public repo and build feature branches from it.

```powershell
git fetch upstream
```

If you want to work from the public repo's `staging` branch:

```powershell
git checkout -b cleanup/remove-railway-kv upstream/staging
```

If you want to work from the public repo's default branch instead:

```powershell
git checkout -b cleanup/remove-railway-kv upstream/main
```

## 3. Use branches, not direct `staging`

Avoid working directly on `staging` or pushing there unless you have explicit permission.

Preferred branch pattern:

- `cleanup/remove-railway-kv`
- `fix/secret-cleanup`
- `feature/<short-description>`

## 4. Make safe cleanup commits

If you need to remove a sensitive file like `railway_kv.txt`:

```powershell
git rm --cached railway_kv.txt
```

Then add a `.gitignore` rule:

```powershell
Add-Content .gitignore "railway_kv.txt"
```

Commit safely:

```powershell
git add .gitignore
git commit -m "Remove exposed Railway KV file from repo tracking and ignore it"
```

## 5. Push only to your fork

Push your branch to your fork remote, not the upstream repo:

```powershell
git push fork cleanup/remove-railway-kv
```

If you rewrite history on your fork, use force-with-lease:

```powershell
git push fork cleanup/remove-railway-kv --force-with-lease
```

## 6. Open a pull request from your fork

Create a PR from `fork/cleanup/remove-railway-kv` into `upstream/staging` or `upstream/main` as appropriate.

This is the safest way to contribute changes back to a public project.

## 7. Rewriting history safely

Only rewrite history if you need to remove a secret from commit history.

### Use `git-filter-repo` when needed

```powershell
python -m git_filter_repo --path railway_kv.txt --invert-paths
```

Then push the rewritten branch to your fork:

```powershell
git push fork cleanup/remove-railway-kv --force-with-lease
```

### Important

- Do not force-push to `upstream` unless you have explicit maintainership permission.
- If you lack write access, keep the rewrite local or on your fork only.

## 8. Secret and environment file policy

Never commit real secrets into tracked files.

- Use `.env`, `.env.local`, or your secret manager for local runtime keys.
- Keep only placeholder values in `.env.example` or repo docs.
- Add secret files to `.gitignore`.

Example ignore rules:

```text
.env
.env.local
.env.*
railway_kv.txt
```

## 9. Verify cleanup

After removing a file from history or tracking, verify there are no remaining leak patterns:

```powershell
Get-ChildItem -Recurse -File | Select-String -Pattern 'API[_-]?KEY|SECRET|PASSWORD|TOKEN|NEARAI|OPENAI|OPENROUTER|AUTH' -SimpleMatch
```

## 10. If you see conflicts in `staging`

Conflicts happen if your local branch diverges from upstream.

The safest approach:

1. Fetch upstream:
   ```powershell
git fetch upstream
```
2. Rebase your branch onto upstream/staging:
   ```powershell
git checkout cleanup/remove-railway-kv
git rebase upstream/staging
```
3. Resolve conflicts, then continue:
   ```powershell
git add <resolved-files>
git rebase --continue
```

Then force-push the updated branch to your fork:

```powershell
git push fork cleanup/remove-railway-kv --force-with-lease
```

## 11. Final best-practice reminders

- Keep public repo remotes read-only (`upstream`) unless you have permission.
- Work in feature branches and use PRs.
- Do not push sensitive files to git.
- Prefer `--force-with-lease` over `--force` when rewriting history.
- Keep your fork synced with upstream before starting new work.
