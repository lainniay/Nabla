use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc,
};

pub(crate) async fn read_stderr(
    stderr: tokio::process::ChildStderr,
    stderr_tx: mpsc::Sender<String>,
) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if stderr_tx.send(line).await.is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                let _ = stderr_tx
                    .send(format!("failed reading Pi stderr: {error}"))
                    .await;
                break;
            }
        }
    }
}
