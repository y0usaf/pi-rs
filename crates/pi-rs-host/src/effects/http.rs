use std::collections::HashMap;

use futures_util::StreamExt as _;
use tokio::sync::mpsc;

use super::{EffectError, EffectOptions, EffectTimeout, RequestContext, ResourceLease};

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub options: EffectOptions,
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub stream: HttpStream,
}

#[derive(Debug)]
pub struct HttpStream {
    rx: mpsc::Receiver<Result<Vec<u8>, EffectError>>,
    pub capacity: usize,
}

impl HttpStream {
    pub async fn next(&mut self) -> Option<Result<Vec<u8>, EffectError>> {
        self.rx.recv().await
    }
}

fn deadline(timeout: EffectTimeout) -> Option<tokio::time::Instant> {
    match timeout {
        EffectTimeout::After(duration) => Some(tokio::time::Instant::now() + duration),
        EffectTimeout::Disabled => None,
    }
}

async fn deadline_wait(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

pub async fn start(
    request: HttpRequest,
    context: RequestContext,
    lease: ResourceLease,
) -> Result<HttpResponse, EffectError> {
    let deadline = deadline(request.options.timeout);
    let mut builder = reqwest::Client::new().get(&request.url);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    let response = tokio::select! {
        response = builder.send() => response.map_err(|error| EffectError::Http(error.to_string()))?,
        () = context.cancelled() => return Err(EffectError::Cancelled),
        () = deadline_wait(deadline) => return Err(EffectError::Timeout),
    };
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let capacity = request.options.stream_capacity;
    let max_output = request.options.max_output_bytes;
    let (sender, rx) = mpsc::channel(capacity);

    tokio::spawn(async move {
        let _lease = lease;
        let mut body = response.bytes_stream();
        let mut total = 0_usize;
        loop {
            let next = tokio::select! {
                next = body.next() => next,
                () = context.cancelled() => {
                    let _ = sender.try_send(Err(EffectError::Cancelled));
                    return;
                }
                () = deadline_wait(deadline) => {
                    let _ = sender.try_send(Err(EffectError::Timeout));
                    return;
                }
            };
            let Some(next) = next else {
                return;
            };
            let bytes = match next {
                Ok(bytes) => bytes.to_vec(),
                Err(error) => {
                    let _ = sender.try_send(Err(EffectError::Http(error.to_string())));
                    return;
                }
            };
            total = total.saturating_add(bytes.len());
            if total > max_output {
                let _ = sender.try_send(Err(EffectError::OutputLimit(max_output)));
                return;
            }
            let sent = tokio::select! {
                sent = sender.send(Ok(bytes)) => sent.is_ok(),
                () = context.cancelled() => false,
                () = deadline_wait(deadline) => false,
            };
            if !sent {
                return;
            }
        }
    });

    Ok(HttpResponse {
        status,
        headers,
        stream: HttpStream { rx, capacity },
    })
}
