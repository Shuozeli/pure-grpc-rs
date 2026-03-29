// E2E test: Connect to WebTransport echo server, send messages, verify echo.
//
// The test page is served locally (not from the H3 server) to avoid
// QUIC TLS issues with self-signed certs during page load.
// The WebTransport connection uses serverCertificateHashes for self-signed cert.
import puppeteer from 'puppeteer-core';
import { readFileSync } from 'fs';
import { createServer } from 'http';

const WT_PORT = 4433;
const HTTP_PORT = 8765;
const TIMEOUT = 15_000;

// The cert hash is passed as an env var by the e2e_test.sh script.
const CERT_HASH_B64 = process.env.CERT_HASH_B64;
if (!CERT_HASH_B64) {
    console.error('CERT_HASH_B64 env var required');
    process.exit(1);
}

async function main() {
    // Serve the HTML test page over plain HTTP.
    const html = readFileSync('/app/index.html', 'utf-8')
        .replace('{{CERT_HASH_B64}}', CERT_HASH_B64)
        .replace(/location\.port/g, String(WT_PORT))
        .replace(/location\.hostname/g, "'localhost'");
    const httpServer = createServer((req, res) => {
        res.writeHead(200, { 'content-type': 'text/html' });
        res.end(html);
    });
    await new Promise(resolve => httpServer.listen(HTTP_PORT, resolve));
    console.log(`HTML server on http://localhost:${HTTP_PORT}`);

    console.log('Launching headless Chrome...');
    const browser = await puppeteer.launch({
        executablePath: process.env.PUPPETEER_EXECUTABLE_PATH || '/usr/bin/chromium',
        headless: 'shell',
        args: [
            '--no-sandbox',
            '--disable-gpu',
            '--enable-quic',
            // WebTransport with self-signed certs needs this.
            '--webtransport-developer-mode',
            '--enable-features=WebTransport',
        ],
    });

    try {
        const page = await browser.newPage();
        page.on('console', msg => console.log(`[browser] ${msg.text()}`));
        page.on('pageerror', err => console.error(`[browser error] ${err.message}`));

        console.log(`Navigating to http://localhost:${HTTP_PORT}/...`);
        await page.goto(`http://localhost:${HTTP_PORT}/`, {
            waitUntil: 'domcontentloaded',
            timeout: TIMEOUT,
        });

        console.log('Waiting for test result...');
        await page.waitForFunction(
            () => document.body.dataset.testResult !== undefined,
            { timeout: TIMEOUT }
        );

        const result = await page.evaluate(() => document.body.dataset.testResult);
        const statusText = await page.evaluate(() => document.getElementById('status').textContent);

        console.log(`Status: ${statusText}`);
        console.log(`Result: ${result}`);

        if (result === 'PASS') {
            console.log('WebTransport E2E test PASSED!');
            process.exit(0);
        } else {
            console.error(`WebTransport E2E test FAILED: ${statusText}`);
            process.exit(1);
        }
    } finally {
        await browser.close();
        httpServer.close();
    }
}

main().catch(err => {
    console.error('E2E test error:', err.message);
    process.exit(1);
});
