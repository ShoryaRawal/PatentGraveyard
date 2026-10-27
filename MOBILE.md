# GraveTender — mobile assistant

GraveTender is the phone-sized companion to the patent desk. It is deliberately
**not** the desk in a smaller window: no graph, no patent table, no dossier
editor, and no seed workflow. It does four things:

- answer questions grounded in the **current patent session**;
- turn the session into **structured reports**;
- keep those reports on the server in SQLite; and
- share a report as Markdown through the native share sheet.

The desktop terminal remains the place where the session is built and explored.

## Repository layout

| Path | Purpose |
|---|---|
| `mobile-ui/` | Separate Leptos **CSR** app, built by Trunk into `mobile-ui/dist` |
| `src-tauri/` | Tauri 2 shell that bundles `mobile-ui/dist` into the native app |
| `src/mobile.rs` | Authenticated mobile API, SQLite report store |
| `src/agent.rs` | Chat agent plus the bounded report-generation prompt |
| `deploy/` | systemd unit, Caddyfile, environment template, backup script |
| `MOBILE.md` | This guide |

The existing root app (`src/app.rs`, `src/islands.rs`, …) is unchanged aside from
the new `/api/mobile` routes. The desktop is still an SSR/islands app; the mobile
frontend is a separate CSR bundle because the islands app is not a standalone
static shell.

## Runtime shape

```text
Phone (Tauri WebView)
  │  HTTPS + Bearer token
  ▼
Caddy  ──/api/mobile/*──►  Axum server  ──►  Groq (server-side key)
  │                         │
  └──everything else──►     ├── in-memory patent session
       (basic auth/VPN)     └── SQLite reports
```

The mobile app **never** calls SerpApi. That keeps the mobile binary from
spending credits, and keeps `SERPAPI_KEY` and `GROQ_API_KEY` on the server.
The live patent session is still in memory and resets when the server restarts;
saved reports survive.

## Mobile API

All routes except `/health` require:

```http
Authorization: Bearer <MOBILE_API_TOKEN>
```

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/mobile/health` | Unauthenticated liveness/storage/agent check |
| `GET` | `/api/mobile/bootstrap` | Dead-only session summary, recent patents, reports |
| `POST` | `/api/mobile/chat` | `{ "messages": [{ "role": "user", "content": "…" }] }` |
| `POST` | `/api/mobile/reports` | `{ "kind": "session_memo\|patent_onepager\|next_steps\|founder_questions", "patent_id": "…", "topic": "…" }` |
| `GET` | `/api/mobile/reports` | Report summaries |
| `GET` | `/api/mobile/reports/{id}` | Full report with Markdown body |
| `DELETE` | `/api/mobile/reports/{id}` | Delete a report |

`bootstrap` returns only US-lapsed patents in `session.recent`, matching the
desktop rule that in-force patents are not displayed. Report generation is
session-grounded: the model receives the session digest, up to twelve lapsed
patent cards, and focused excerpts, and every factual citation is validated
against the corpus.

## Run it locally

Terminal 1 — backend (the same server as the desktop):

```bash
./run.sh ./target/release/patent-graveyard
# or:
./run.sh cargo run --no-default-features --features ssr
```

`run.sh` sets a development token, `grave-tender-dev`, and the SQLite path
`./gravetender.db`. Override either in the environment.

Terminal 2 — mobile frontend preview:

```bash
cd mobile-ui
trunk serve --port 8080
```

Open `http://127.0.0.1:8080`, then **Settings**:

- Backend URL: `http://127.0.0.1:3000`
- Shared token: `grave-tender-dev`

The mobile API has permissive CORS because the Tauri custom-scheme origin is not
practical to enumerate; the bearer token is the actual gate. The desktop
`/api/search` and `/api/agent` routes do **not** currently have their own auth.

### Desktop Tauri shell

```bash
# terminal 1
./run.sh ./target/release/patent-graveyard

# terminal 2
cargo tauri dev
```

The Tauri config starts only the Trunk dev server (`beforeDevCommand`), so the
Axum server must already be running. The Tauri window is sized like a phone for
desktop preview.

## Native prerequisites

Current machine state (as of this scaffold): full Xcode is **not** installed;
only Command Line Tools are active. Java, Android SDK/NDK, Android Studio, and
the iOS/Android Rust targets are also missing. Code and the web build work now;
native device/simulator builds need the following.

### Shared

```bash
rustup target add wasm32-unknown-unknown
cargo install tauri-cli --version "^2" --locked
cargo install trunk
# binaryen (wasm-opt): brew install binaryen
```

### iOS

1. Install **Xcode** from the App Store (not just Command Line Tools).
2. Select it and accept the licence:
   ```bash
   sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
   sudo xcodebuild -license accept
   ```
3. Add the Rust targets:
   ```bash
   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
   ```
4. Join the Apple Developer Program for TestFlight/internal distribution. A free
   personal team works for a simulator/device debug build but not TestFlight.

### Android

1. Install Android Studio, then the SDK, platform tools, and an NDK (the Tauri
   Android guide lists the current NDK version).
2. Install JDK 17 and set `JAVA_HOME`.
3. Set `ANDROID_HOME`/`ANDROID_SDK_ROOT`, and put `platform-tools` on `PATH`.
4. Add the Rust targets:
   ```bash
   rustup target add aarch64-linux-android armv7-linux-androideabi \
     i686-linux-android x86_64-linux-android
   ```
5. Accept SDK licences:
   ```bash
   sdkmanager --licenses
   ```

## Generate the native projects

Tauri mobile keeps platform projects separate. From the repository root:

```bash
cargo tauri ios init
cargo tauri android init
```

This creates `src-tauri/gen/apple` and `src-tauri/gen/android`. Commit or back
up those projects once created; signing settings live there. The bundle
identifier is `com.patentgraveyard.gravetender`.

### Development runs

```bash
# iOS simulator (choose a simulator when prompted)
cargo tauri ios dev

# Android emulator
cargo tauri android dev

# Physical device: let Tauri pick the public host for the dev server
cargo tauri ios dev --host
cargo tauri android dev --host
```

`beforeDevCommand` binds Trunk to `0.0.0.0:8080` so a phone on the same LAN can
reach the dev frontend. The API server still needs `GRAVE_TENDER_ADDR=0.0.0.0:3000`
for that case, and its URL goes in the app's Settings.

API base URLs in the app's Settings:

- iOS simulator: `http://127.0.0.1:3000`
- Android emulator: `http://10.0.2.2:3000`
- Physical device on the LAN: start the server with
  `GRAVE_TENDER_ADDR=0.0.0.0:3000`, then use `http://<mac-lan-ip>:3000`.
  iOS ATS may block plain HTTP; HTTPS through Caddy is the reliable path.

The app defaults to `127.0.0.1` on iOS and `10.0.2.2` on Android. A build can
bake a different default with `GRAVE_TENDER_API` in the environment before
`trunk build`:

```bash
cd mobile-ui
GRAVE_TENDER_API=https://api.example.com trunk build --release
```

The Settings screen still overrides it, so the default is a convenience, not a
secret.

## Internal distribution

### iOS / TestFlight

```bash
cd /path/to/project
cargo tauri ios build --export-method release-testing
```

This builds the IPA with the signing team configured in Xcode. Upload it with
**Transporter** (or Xcode Organizer), create the app in App Store Connect if
needed, add internal testers, and distribute through TestFlight. Increment the
version/build for each upload.

For App Store distribution later, use `--export-method app-store-connect`.

### Android / signed APK

Open `src-tauri/gen/android` in Android Studio and use **Build → Generate Signed
Bundle or APK → APK**, or configure a release signing config in Gradle. A signed
APK can be installed directly or hosted on an internal HTTPS link; the Play
Console is not required.

CLI build (unsigned or with Gradle signing configured):

```bash
cargo tauri android build --apk --split-per-abi
```

Outputs land under `src-tauri/gen/android/app/build/outputs/apk/`.

## AWS deployment

A single small instance is enough for an internal pilot. The reports database
is SQLite and the session is in memory.

### 1. Instance and network

- Ubuntu 24.04 LTS, `t4g.small` (ARM) or `t3.small`, 20 GB gp3.
- Elastic IP.
- Security group: `22` from your IP only, `80`/`443` from the internet.
- DNS `A` record for `api.example.com` → Elastic IP.

### 2. Host packages

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev sqlite3 caddy curl
# Rust via rustup, then:
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
# binaryen: sudo apt install binaryen
```

### 3. Build and install

```bash
sudo useradd --system --home /opt/gravetender --shell /usr/sbin/nologin gravetender
sudo mkdir -p /opt/gravetender /var/lib/gravetender
sudo chown -R gravetender:gravetender /opt/gravetender /var/lib/gravetender

# From the repository checkout on the instance:
trunk build --release
cargo build --release --no-default-features --features ssr
sudo install -m 0755 target/release/patent-graveyard /opt/gravetender/
sudo cp -r dist Cargo.toml /opt/gravetender/
sudo chown -R gravetender:gravetender /opt/gravetender
```

`Cargo.toml` is needed at runtime because `get_configuration(Some("Cargo.toml"))`
reads the Leptos site configuration from it.

### 4. Environment and service

```bash
sudo install -m 0640 -g gravetender deploy/gravetender.env.example /etc/gravetender.env
sudoedit /etc/gravetender.env
# generate a real token:
# openssl rand -hex 32

sudo cp deploy/gravetender.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now gravetender
sudo systemctl status gravetender
```

### 5. Caddy and HTTPS

```bash
sudo cp deploy/Caddyfile /etc/caddy/Caddyfile
sudoedit /etc/caddy/Caddyfile
# replace api.example.com and the bcrypt hash
caddy hash-password --plaintext 'choose-a-password'
sudo systemctl reload caddy
```

Caddy exposes only `/api/mobile/*` without basic auth. The desktop terminal,
including the unauthenticated `/api/search` route, sits behind basic auth. Put
it behind a VPN instead if you prefer.

### 6. Backups

```bash
sudo install -m 0750 deploy/backup.sh /usr/local/bin/gravetender-backup
sudo /usr/local/bin/gravetender-backup
# cron: 17 3 * * * /usr/local/bin/gravetender-backup
```

For off-instance retention, copy the backup to S3 and enable EBS snapshots. The
backup script uses SQLite's online `.backup`, so it is safe while the service is
running.

## Mobile configuration in production

After deployment, set the app's Settings to:

- Backend URL: `https://api.example.com`
- Shared token: the value in `/etc/gravetender.env`

The token is stored in the WebView's local storage. That is acceptable for an
internal shared-token pilot, but it is **not** a hardware-backed secret store.
Before widening distribution, replace the shared token with per-device tokens
and use Keychain/Android Keystore (or a Tauri secure-storage plugin), and add
rate limiting/audit at the proxy or API layer.

## Known limits

- **Session is ephemeral.** A server restart clears the patent session; reports
  remain. Reports always describe the session at generation time.
- **One shared token.** No per-user identity, revocation list, or audit trail.
- **No PDF yet.** Reports are Markdown with native share and copy; PDF is later.
- **No mobile search/ingest.** The phone does not spend SerpApi credits.
- **No offline mode.** Chat and report generation require the backend.
- **Research signalling, not legal advice.** Status comes from Google Patents
  data; confirm with the USPTO before a freedom-to-operate decision.
