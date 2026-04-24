use httpdate::parse_http_date;
use rand::Rng;
use reqwest::{Response, StatusCode, header::HeaderMap};
use std::{future::Future, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RetryOptions {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter_ratio: f64,
}

impl Default for RetryOptions {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 500,
            max_backoff_ms: 8_000,
            jitter_ratio: 0.25,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetryError {
    Aborted,
    Message(String),
}

pub(crate) async fn send_request_with_retry<F, Fut>(
    options: RetryOptions,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
    mut send: F,
) -> Result<Response, RetryError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Response, reqwest::Error>>,
{
    let max_attempts = options.max_attempts.max(1);
    let mut attempt = 1;

    loop {
        if is_signal_aborted(signal) {
            return Err(RetryError::Aborted);
        }

        let send_future = send();
        tokio::pin!(send_future);
        let response = if let Some(signal) = signal.as_mut() {
            tokio::select! {
                response = &mut send_future => response,
                _ = wait_for_abort(signal) => return Err(RetryError::Aborted),
            }
        } else {
            send_future.await
        };

        match response {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                if should_retry_status(status) && attempt < max_attempts {
                    let delay = retry_delay(response.headers(), options, attempt);
                    sleep_with_abort(delay, signal).await?;
                    attempt += 1;
                    continue;
                }

                let body = read_error_body(response, signal).await?;
                let detail = if body.is_empty() {
                    format!("HTTP request failed with status {status}")
                } else {
                    format!("HTTP request failed with status {status}: {body}")
                };
                return Err(RetryError::Message(detail));
            }
            Err(error) => {
                if is_signal_aborted(signal) {
                    return Err(RetryError::Aborted);
                }

                if attempt < max_attempts {
                    let delay = retry_delay(&HeaderMap::new(), options, attempt);
                    sleep_with_abort(delay, signal).await?;
                    attempt += 1;
                    continue;
                }

                return Err(RetryError::Message(format!("HTTP request failed: {error}")));
            }
        }
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_delay(headers: &HeaderMap, options: RetryOptions, attempt: u32) -> Duration {
    parse_retry_after(headers).unwrap_or_else(|| default_backoff(options, attempt))
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();

    if let Ok(seconds) = value.parse::<f64>()
        && seconds.is_finite()
        && seconds >= 0.0
    {
        return Some(Duration::from_secs_f64(seconds));
    }

    let retry_at = parse_http_date(value).ok()?;
    Some(
        retry_at
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default(),
    )
}

fn default_backoff(options: RetryOptions, attempt: u32) -> Duration {
    let retry_index = attempt.saturating_sub(1);
    let backoff_ms = ((options.initial_backoff_ms as f64) * 2f64.powi(retry_index as i32))
        .min(options.max_backoff_ms as f64);
    let jitter_cap = options.jitter_ratio.clamp(0.0, 1.0);
    let jitter_multiplier = if jitter_cap == 0.0 {
        1.0
    } else {
        1.0 - rand::thread_rng().gen_range(0.0..jitter_cap)
    };

    Duration::from_secs_f64((backoff_ms * jitter_multiplier) / 1000.0)
}

async fn read_error_body(
    response: Response,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<String, RetryError> {
    let body_future = response.text();
    tokio::pin!(body_future);

    if let Some(signal) = signal.as_mut() {
        tokio::select! {
            body = &mut body_future => Ok(body.unwrap_or_default()),
            _ = wait_for_abort(signal) => Err(RetryError::Aborted),
        }
    } else {
        Ok(body_future.await.unwrap_or_default())
    }
}

async fn sleep_with_abort(
    duration: Duration,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<(), RetryError> {
    if let Some(signal) = signal.as_mut() {
        tokio::select! {
            _ = tokio::time::sleep(duration) => Ok(()),
            _ = wait_for_abort(signal) => Err(RetryError::Aborted),
        }
    } else {
        tokio::time::sleep(duration).await;
        Ok(())
    }
}

fn is_signal_aborted(signal: &Option<tokio::sync::watch::Receiver<bool>>) -> bool {
    signal
        .as_ref()
        .map(|signal| *signal.borrow())
        .unwrap_or(false)
}

async fn wait_for_abort(signal: &mut tokio::sync::watch::Receiver<bool>) {
    while !*signal.borrow() {
        if signal.changed().await.is_err() {
            return;
        }
    }
}
