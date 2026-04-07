# Deploying IronClaw to Railway

## Quick Deploy

1. **Install Railway CLI**
   ```bash
   npm i -g @railway/cli
   ```

2. **Login and Link Project**
   ```bash
   railway login
   railway link
   ```

3. **Deploy**
   ```bash
   railway up --detach
   ```

4. **Set Environment Variables**
   ```bash
   railway variable set "DATABASE_BACKEND=libsql"
   railway variable set "LLM_BACKEND=openrouter"
   railway variable set "OPENROUTER_API_KEY=sk-or-v1-..."
   railway variable set "OPENROUTER_MODEL=qwen/qwen3.6-plus:free"
   railway variable set "SECRETS_MASTER_KEY=$(openssl rand -hex 32)"
   railway variable set "GATEWAY_ENABLED=true"
   railway variable set "GATEWAY_HOST=0.0.0.0"
   railway variable set "GATEWAY_AUTH_TOKEN=your-secure-token"
   railway variable set "ONBOARD_COMPLETED=true"
   railway variable set "PORT=3000"
   ```

5. **Redeploy**
   ```bash
   railway redeploy --yes
   ```

6. **Access Your App**
   ```bash
   railway domain
   ```
   Then open: `https://your-project.up.railway.app/?token=your-auth-token`

## Configuration

See `.env.railway` for all required environment variables.

## Sandbox Mode

Note: Railway's default deployment environment does not support nested Docker (Docker-in-Docker). Sandbox-based tasks and `full_job` routines will fail unless using a custom Docker-enabled environment.

## Known Issues

See `ISSUES.md` for deployment troubleshooting.
