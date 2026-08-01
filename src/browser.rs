use std::process::Stdio;

use tokio::process::Command;

const MAX_AUTH_URL_LEN: usize = 16 * 1024;

pub fn is_safe_web_url(url: &str) -> bool {
    url.len() <= MAX_AUTH_URL_LEN
        && (url.starts_with("https://") || url.starts_with("http://"))
        && !url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

pub async fn open_url(url: &str) -> Result<(), String> {
    if !is_safe_web_url(url) {
        return Err("refused an invalid or unsafe URL".to_owned());
    }

    let mut command = platform_open_command(url);
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("URL opener exited with status {status}"))
    }
}

#[cfg(target_os = "macos")]
fn platform_open_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(target_os = "windows")]
fn platform_open_command(url: &str) -> Command {
    let mut command = Command::new("rundll32");
    command.arg("url.dll,FileProtocolHandler").arg(url);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_open_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allows_terminal_safe_http_urls() {
        assert!(is_safe_web_url(
            "https://auth.openai.com/oauth/authorize?state=abc"
        ));
        assert!(is_safe_web_url("http://127.0.0.1:1455/auth/callback"));
        assert!(!is_safe_web_url("javascript:alert(1)"));
        assert!(!is_safe_web_url("https://example.com/\u{1b}]8;;evil"));
        assert!(!is_safe_web_url("https://example.com/has space"));
    }

    #[test]
    fn platform_command_passes_the_url_as_one_argument() {
        let url = "https://example.com/path?one=1&two=2";
        let command = platform_open_command(url);
        let arguments = command
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(arguments.last().map(String::as_str), Some(url));
    }
}
