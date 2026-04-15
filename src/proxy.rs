use std::io::Cursor;

use tiny_http::{Header, Response, Server};
use url::Url;

use crate::get_mapping;

const EMAIL_HEADER: &str = "X-Sync-User-Email";

pub fn run(port: u16, upstream_base: &str) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    eprintln!("[lzc-sync] proxy listening on {addr}");

    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(false)
        .build()?;

    for mut request in server.incoming_requests() {
        let result = handle_request(&client, &mut request, upstream_base);
        match result {
            Ok(response) => {
                if let Err(e) = request.respond(response) {
                    eprintln!("[lzc-sync] respond error: {e}");
                }
            }
            Err(e) => {
                eprintln!("[lzc-sync] request error: {e}");
                let resp = Response::from_string(format!("proxy error: {e}"))
                    .with_status_code(502);
                let _ = request.respond(resp);
            }
        }
    }

    Ok(())
}

fn handle_request(
    client: &reqwest::blocking::Client,
    request: &mut tiny_http::Request,
    upstream_base: &str,
) -> Result<Response<Cursor<Vec<u8>>>, Box<dyn std::error::Error>> {
    // 从 URL query 参数中提取 client_id
    let request_url = format!("http://localhost{}", request.url());
    let parsed = Url::parse(&request_url)?;
    let client_id = parsed
        .query_pairs()
        .find(|(k, _)| k == "client_id")
        .map(|(_, v)| v.to_string());

    // 查找 email
    let email = client_id
        .as_deref()
        .and_then(|id| get_mapping().and_then(|m| m.lookup(id)));

    if let Some(email) = email {
        eprintln!("[lzc-sync] sync request from: {email} (client_id: {})", client_id.as_deref().unwrap_or("?"));
    } else {
        eprintln!("[lzc-sync] sync request, unknown user (client_id: {:?})", client_id);
    }

    // 构建上游 URL: 替换 host 部分，保留 path 和 query
    let upstream_url = build_upstream_url(upstream_base, request.url())?;

    // 读取请求 body
    let mut body = Vec::new();
    request.as_reader().read_to_end(&mut body)?;

    // 构建转发请求
    let mut upstream_req = client.post(&upstream_url);

    // 复制原始 headers
    for header in request.headers() {
        let name = header.field.as_str().as_str();
        let value = header.value.as_str();
        // 跳过 hop-by-hop headers 和 host
        if matches!(
            name.to_lowercase().as_str(),
            "host" | "connection" | "transfer-encoding" | "content-length"
        ) {
            continue;
        }
        upstream_req = upstream_req.header(name, value);
    }

    // 添加 email header
    if let Some(email) = email {
        upstream_req = upstream_req.header(EMAIL_HEADER, email);
    }

    // 发送请求
    let upstream_resp = upstream_req.body(body).send()?;

    // 构建响应
    let status = upstream_resp.status().as_u16();
    let resp_headers: Vec<Header> = upstream_resp
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let name_str = name.as_str();
            if matches!(
                name_str.to_lowercase().as_str(),
                "transfer-encoding" | "connection"
            ) {
                return None;
            }
            let value_str = value.to_str().ok()?;
            Header::from_bytes(name_str.as_bytes(), value_str.as_bytes()).ok()
        })
        .collect();

    let resp_body = upstream_resp.bytes()?.to_vec();
    let mut response = Response::from_data(resp_body).with_status_code(status);
    for header in resp_headers {
        response.add_header(header);
    }

    Ok(response)
}

/// 将本地代理 URL 转换为上游 Google URL
/// 输入: /chrome-sync/command/?client=Google+Chrome&client_id=xxx
/// 输出: https://clients4.google.com/chrome-sync/command/?client=Google+Chrome&client_id=xxx
fn build_upstream_url(
    upstream_base: &str,
    local_path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let base = Url::parse(upstream_base)?;

    // local_path: /chrome-sync/command/?client=...
    // 去掉前缀 /chrome-sync，保留 /command/?...
    let stripped = local_path
        .strip_prefix("/chrome-sync")
        .unwrap_or(local_path);

    let base_str = base.as_str().trim_end_matches('/');
    Ok(format!("{base_str}{stripped}"))
}
