#!/usr/bin/env bash
# E2E test: WebTransport echo via headless Chrome.
#
# 1. Build and start the WebTransport echo server.
# 2. Use Playwright + system Chrome to connect via WebTransport.
# 3. Send messages, verify echo response.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PORT=4433
PW_DIR="/tmp/pw-test"

echo "=== Building WebTransport echo server ==="
cd "$PROJECT_ROOT"
cargo build -p webtransport-echo --bin wt-echo-server 2>&1

echo "=== Starting server on port $PORT ==="
RUST_LOG=info "$PROJECT_ROOT/target/debug/wt-echo-server" > /tmp/wt-e2e.log 2>&1 &
SERVER_PID=$!

cleanup() {
    echo "=== Cleaning up ==="
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
    rm -f /tmp/wt-e2e.log
}
trap cleanup EXIT

# Wait for server to start and extract cert hash.
for i in $(seq 1 20); do
    if grep -q "base64" /tmp/wt-e2e.log 2>/dev/null; then break; fi
    sleep 0.5
done

CERT_HASH_B64=$(grep "base64" /tmp/wt-e2e.log | grep -oP '(?<=base64\): ).*' | head -1)
if [ -z "$CERT_HASH_B64" ]; then
    echo "ERROR: Could not extract cert hash"
    cat /tmp/wt-e2e.log
    exit 1
fi
echo "=== Cert hash: $CERT_HASH_B64 ==="

# Ensure Playwright is installed.
if [ ! -d "$PW_DIR/node_modules" ]; then
    echo "=== Installing Playwright ==="
    mkdir -p "$PW_DIR"
    cd "$PW_DIR"
    npm init -y > /dev/null 2>&1
    npm install playwright > /dev/null 2>&1
fi

echo "=== Running WebTransport E2E test ==="
cd "$PW_DIR"

# Write the test inline.
cat > wt_e2e.mjs << ENDTEST
import { chromium } from 'playwright';
import { createServer } from 'http';

const CERT = '${CERT_HASH_B64}';
const html = \`<html><body><script>
(async () => {
    document.title = 'running';
    try {
        const hash = Uint8Array.from(atob('\${CERT}'), c => c.charCodeAt(0));
        const t = new WebTransport('https://localhost:${PORT}/wt', {
            serverCertificateHashes: [{ algorithm: 'sha-256', value: hash.buffer }],
        });
        await t.ready;
        const s = await t.createBidirectionalStream();
        const w = s.writable.getWriter();
        await w.write(new TextEncoder().encode('hello'));
        await w.close();
        const r = s.readable.getReader();
        const {value} = await r.read();
        document.title = 'GOT:' + new TextDecoder().decode(value);
        t.close();
    } catch(e) {
        document.title = 'FAIL:' + e.message;
    }
})();
</script></body></html>\`;

const server = createServer((req, res) => { res.writeHead(200, {'content-type':'text/html'}); res.end(html); });
await new Promise(r => server.listen(9876, r));

const browser = await chromium.launch({
    executablePath: '/usr/bin/chromium-browser',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--enable-quic', '--webtransport-developer-mode'],
});
const page = await browser.newPage();
page.on('console', msg => console.log('[browser]', msg.text()));
await page.goto('http://localhost:9876/');
await page.waitForFunction(() => document.title !== '' && document.title !== 'running', { timeout: 15000 });
const title = await page.title();
await browser.close();
server.close();

if (title === 'GOT:echo:hello') {
    console.log('WebTransport echo: PASS');
    process.exit(0);
} else {
    console.error('WebTransport echo: FAIL -', title);
    process.exit(1);
}
ENDTEST

node wt_e2e.mjs

echo ""
echo "=== E2E TEST PASSED ==="
