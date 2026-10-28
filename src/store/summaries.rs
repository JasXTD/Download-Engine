use super::*;
use serde_json::{Value, json};

impl Store {
    /// Count and page in one read snapshot. Only the requested page's job JSON
    /// is decoded; queue membership is joined without per-job connections.
    pub(crate) fn download_summaries(
        &self,
        queue: Option<&str>,
        status: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Value> {
        anyhow::ensure!(
            (1..=100).contains(&limit) && offset <= 1_000_000,
            "Invalid download page"
        );
        anyhow::ensure!(
            status.is_none_or(|s| matches!(
                s,
                "queued" | "downloading" | "paused" | "complete" | "failed"
            )),
            "Invalid download status"
        );
        let mut filters = Vec::new();
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(queue) = queue {
            filters.push("q.queue_id=?");
            args.push(queue.to_owned().into());
        }
        if let Some(status) = status {
            filters.push("json_extract(j.data,'$.status')=?");
            args.push(status.to_owned().into());
        }
        let filter = if filters.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", filters.join(" AND "))
        };
        let from = format!("FROM jobs j LEFT JOIN job_queues q ON q.job_id=j.id{filter}");
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(
            &format!("SELECT COUNT(*) {from}"),
            rusqlite::params_from_iter(&args),
            |row| row.get(0),
        )?;
        let total = usize::try_from(total)?;
        args.push((limit as i64).into());
        args.push((offset as i64).into());
        let items = {
            let mut stmt = tx.prepare(&format!("SELECT j.id,j.data,q.queue_id {from} ORDER BY j.created DESC,j.rowid DESC LIMIT ? OFFSET ?"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(&args), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.map(|row| {
                let (id, data, queue) = row?;
                let job = super::decode_job(&id, &data)?;
                Ok(crate::control::job_summary_in_queue(&job, queue))
            })
            .collect::<Result<Vec<_>>>()?
        };
        tx.commit()?;
        let next = offset + items.len();
        Ok(
            json!({"items":items,"total":total,"next_offset":if next < total {Some(next)} else {None}}),
        )
    }
}

#[cfg(test)]
mod tests;
