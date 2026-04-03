//! ChannelStateStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_text};
use crate::db::ChannelStateStore;
use crate::error::DatabaseError;

use chrono::Utc;

#[async_trait]
impl ChannelStateStore for LibSqlBackend {
    async fn channel_state_read(
        &self,
        channel_name: &str,
        path: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT content FROM channel_state WHERE channel_name = ?1 AND path = ?2",
                params![channel_name, path],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_text(&row, 0))),
            None => Ok(None),
        }
    }

    async fn channel_state_write(
        &self,
        channel_name: &str,
        path: &str,
        content: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                INSERT INTO channel_state (channel_name, path, content, updated_at)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT (channel_name, path) DO UPDATE SET
                    content = excluded.content,
                    updated_at = ?4
            "#,
            params![channel_name, path, content, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn channel_state_delete(
        &self,
        channel_name: &str,
        path: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM channel_state WHERE channel_name = ?1 AND path = ?2",
                params![channel_name, path],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn channel_state_list(
        &self,
        channel_name: &str,
        prefix: Option<&str>,
    ) -> Result<Vec<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = match prefix {
            Some(prefix) => {
                let pattern = format!("{}%", prefix);
                conn.query(
                    "SELECT path FROM channel_state WHERE channel_name = ?1 AND path LIKE ?2 ORDER BY path",
                    params![channel_name, pattern],
                )
                .await
            }
            None => {
                conn.query(
                    "SELECT path FROM channel_state WHERE channel_name = ?1 ORDER BY path",
                    params![channel_name],
                )
                .await
            }
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            paths.push(get_text(&row, 0));
        }
        Ok(paths)
    }
}
