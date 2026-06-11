use hyper::body::{Bytes, HttpBody};
use hyper::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response};
use std::convert::Infallible;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use tower_http::services::ServeDir;

/// Path the injected script connects to for rebuild notifications.
const LIVERELOAD_PATH: &str = "/__squid/livereload";

/// Injected into every served HTML page; reloads the browser when the
/// watcher finishes a rebuild.
const RELOAD_SCRIPT: &str = "\n<script>(() => { new EventSource('/__squid/livereload')\
                             .onmessage = () => location.reload(); })();</script>\n";

/// Serve the output folder, injecting a live-reload script into HTML pages.
/// A message on `reload` tells every connected browser to refresh.
pub fn serve(port: u16, folder: &str, reload: broadcast::Sender<()>) -> JoinHandle<()> {
    let folder = folder.to_string();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));

    tokio::task::spawn(async move {
        let make_svc = make_service_fn(move |_conn| {
            let folder = folder.clone();
            let reload = reload.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    handle(req, folder.clone(), reload.clone())
                }))
            }
        });

        hyper::Server::bind(&addr)
            .serve(make_svc)
            .await
            .expect("server error")
    })
}

async fn handle(
    req: Request<Body>,
    folder: String,
    reload: broadcast::Sender<()>,
) -> Result<Response<Body>, Infallible> {
    if req.uri().path() == LIVERELOAD_PATH {
        return Ok(livereload_stream(reload));
    }

    match ServeDir::new(&folder).oneshot(req).await {
        Ok(res) => Ok(with_reload_script(res).await),
        Err(_) => unreachable!("ServeDir is infallible"),
    }
}

/// Server-sent events stream that emits one message per rebuild.
fn livereload_stream(reload: broadcast::Sender<()>) -> Response<Body> {
    let mut rx = reload.subscribe();
    let (mut sender, body) = Body::channel();

    tokio::spawn(async move {
        // SSE comment to open the stream immediately
        if sender
            .send_data(Bytes::from_static(b": connected\n\n"))
            .await
            .is_err()
        {
            return;
        }
        loop {
            match rx.recv().await {
                Ok(()) => {
                    if sender
                        .send_data(Bytes::from_static(b"data: reload\n\n"))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                // the browser only needs the latest rebuild, skipped ones don't matter
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Response::builder()
        .header(CONTENT_TYPE, "text/event-stream")
        .header(CACHE_CONTROL, "no-cache")
        .body(body)
        .expect("static headers are valid")
}

/// Buffer the file response and append the reload script to HTML pages.
/// Buffering is fine here: this server only ever serves a local preview.
async fn with_reload_script<B>(res: Response<B>) -> Response<Body>
where
    B: HttpBody<Data = Bytes>,
{
    let is_html = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/html"))
        .unwrap_or(false);

    let (mut parts, body) = res.into_parts();
    let bytes = match hyper::body::to_bytes(body).await {
        Ok(b) => b,
        Err(_) => {
            return Response::builder()
                .status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("failed to read file"))
                .expect("static response is valid")
        }
    };

    let bytes = if is_html {
        let mut with_script = bytes.to_vec();
        with_script.extend_from_slice(RELOAD_SCRIPT.as_bytes());
        Bytes::from(with_script)
    } else {
        bytes
    };

    parts.headers.insert(CONTENT_LENGTH, bytes.len().into());
    Response::from_parts(parts, Body::from(bytes))
}
