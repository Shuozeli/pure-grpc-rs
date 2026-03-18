#!/bin/bash
# Generate self-signed test certificates for TLS examples.
# Run from the certs/ directory.
set -e

cd "$(dirname "$0")"

# Generate CA
openssl req -x509 -newkey rsa:2048 -keyout ca.key -out ca.crt \
  -days 365 -nodes -subj "/CN=Test CA" 2>/dev/null

# Generate server key + CSR
openssl req -newkey rsa:2048 -keyout server.key -out server.csr \
  -nodes -subj "/CN=localhost" 2>/dev/null

# Sign server cert with CA
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key \
  -CAcreateserial -out server.crt -days 365 \
  -extfile <(echo "subjectAltName=DNS:localhost,IP:127.0.0.1") 2>/dev/null

rm -f server.csr ca.srl

echo "Generated: ca.crt ca.key server.crt server.key"
