// Lightweight proxy: 8046 -> 8045 with path normalization and Bearer token
// Fixes: multiple slashes in path (///v1internal -> /v1internal)
// Also handles /oauth2/v3/tokeninfo for IDE auth checks

const http = require('http');

const LISTEN_PORT = 8046;
const TARGET_HOST = '127.0.0.1';
const TARGET_PORT = 8055;
const BEARER_TOKEN = 'sk-556ae1cc3a46440889de5523cae2c897';

function normalizePath(url) {
  // Split query string from path
  const qIdx = url.indexOf('?');
  let path = qIdx >= 0 ? url.substring(0, qIdx) : url;
  const query = qIdx >= 0 ? url.substring(qIdx) : '';
  
  // Collapse multiple consecutive slashes into one
  path = path.replace(/\/+/g, '/');
  
  // Ensure path starts with /
  if (!path.startsWith('/')) path = '/' + path;
  
  // Rewrite /v1internal:xxx to /v1internal/xxx to avoid Axum route parsing issues
  if (path.startsWith('/v1internal:')) {
    path = path.replace(':', '/');
  }
  
  return path + query;
}

const server = http.createServer((clientReq, clientRes) => {
  const startTime = Date.now();
  const method = clientReq.method;
  const rawUrl = clientReq.url;
  const url = normalizePath(rawUrl);

  console.log('[Proxy] ' + method + ' ' + rawUrl + ' -> normalized: ' + url);

  // Handle /oauth2/v3/tokeninfo and userinfo locally - IDE checks if token is valid
  if (url.startsWith('/oauth2/') && (url.includes('tokeninfo') || url.includes('userinfo'))) {
    console.log('[Proxy] Handling ' + url + ' locally (fake valid response)');
    let fakeInfo;
    if (url.includes('tokeninfo')) {
      fakeInfo = JSON.stringify({
        "issued_to": "antigravity-client",
        "audience": "antigravity-client",
        "scope": "https://www.googleapis.com/auth/cloud-platform",
        "expires_in": 3599,
        "access_type": "offline"
      });
    } else {
      fakeInfo = JSON.stringify({
        "id": "1234567890",
        "email": "local@antigravity",
        "verified_email": true,
        "name": "Antigravity Local User",
        "given_name": "Antigravity",
        "family_name": "Local",
        "picture": "https://lh3.googleusercontent.com/a/default-user",
        "locale": "en"
      });
    }
    clientRes.writeHead(200, {
      'content-type': 'application/json',
      'content-length': Buffer.byteLength(fakeInfo)
    });
    clientRes.end(fakeInfo);
    return;
  }

  // Collect incoming body
  const bodyChunks = [];
  clientReq.on('data', chunk => bodyChunks.push(chunk));
  clientReq.on('end', () => {
    const body = Buffer.concat(bodyChunks);

    // Build headers for upstream
    const headers = {};
    for (const [key, value] of Object.entries(clientReq.headers)) {
      const lower = key.toLowerCase();
      if (lower === 'host' || lower === 'connection' || lower === 'transfer-encoding') continue;
      headers[key] = value;
    }
    headers['Authorization'] = 'Bearer ' + BEARER_TOKEN;
    if (body.length > 0) {
      headers['Content-Length'] = body.length;
    }

    const options = {
      hostname: TARGET_HOST,
      port: TARGET_PORT,
      path: url,  // normalized path
      method: method,
      headers: headers,
      timeout: 300000
    };

    const proxyReq = http.request(options, (proxyRes) => {
      const respChunks = [];
      proxyRes.on('data', chunk => respChunks.push(chunk));
      proxyRes.on('end', () => {
        const respBody = Buffer.concat(respChunks);
        const elapsed = Date.now() - startTime;
        console.log('[Proxy] Response: ' + proxyRes.statusCode + ' (' + respBody.length + ' bytes, ' + elapsed + 'ms)');

        const respHeaders = {};
        for (const [key, value] of Object.entries(proxyRes.headers)) {
          const lower = key.toLowerCase();
          if (lower === 'transfer-encoding' || lower === 'content-length' || lower === 'connection') continue;
          respHeaders[key] = value;
        }
        respHeaders['content-length'] = respBody.length;

        clientRes.writeHead(proxyRes.statusCode, respHeaders);
        clientRes.end(respBody);
      });
    });

    proxyReq.on('error', (err) => {
      console.error('[Proxy] Upstream error: ' + err.message);
      clientRes.writeHead(502, { 'content-type': 'text/plain' });
      clientRes.end('Upstream error: ' + err.message);
    });

    proxyReq.on('timeout', () => {
      console.error('[Proxy] Upstream timeout');
      proxyReq.destroy();
      clientRes.writeHead(504, { 'content-type': 'text/plain' });
      clientRes.end('Gateway Timeout');
    });

    if (body.length > 0) proxyReq.write(body);
    proxyReq.end();
  });
});

server.listen(LISTEN_PORT, '127.0.0.1', () => {
  console.log('[Proxy] Listening on http://127.0.0.1:' + LISTEN_PORT);
  console.log('[Proxy] Forwarding to http://' + TARGET_HOST + ':' + TARGET_PORT);
  console.log('[Proxy] Path normalization: ON (fixes ///v1internal -> /v1internal)');
  console.log('[Proxy] OAuth tokeninfo: handled locally');
});
