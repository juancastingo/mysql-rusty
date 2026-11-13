# 🦀 MySQL-Rusty Workbench

A modern, fast, **100% native Rust** desktop alternative to MySQL Workbench for Linux (Ubuntu X11/Wayland), featuring **schema-aware AI inline autocomplete** and **multi-provider AI / CLI Copilot integration** (`Ollama`, `OpenAI/Compatible`, `Anthropic`, `Gemini`, and local CLI tools like `agy` or `llm`).

---

## ✨ Features Implemented in the Base Foundation

- **100% Native Rust Desktop GUI (`eframe` / `egui` + `glow` OpenGL):** Zero WebView overhead, instant startup, low memory footprint, and dark-mode SQL workbench layout.
- **Connection Manager & Built-in Sandbox Mode:**
  - Connect to any MySQL 8.0+ or MariaDB server with SSL/TLS (`Preferred`, `Required`, `Disabled`) and native SSH port forwarding (`ssh -N -L`).
  - Includes an instant **⚡ Sandbox Demo (`ecommerce_prod`)** connection profile preloaded with `users`, `products`, `orders`, `order_items`, and `payments` so you can test every feature immediately without a local MySQL server.
- **Schema Explorer & ER Overview:**
  - Live introspection of databases, tables, approximate row counts, columns, primary keys (`🔑`), foreign keys (`🔗`), and indexes via `INFORMATION_SCHEMA`.
  - Quick actions (`SELECT *`, `DDL`, `Insert Name`) and an Entity-Relationship (ER) schema diagram tab.
  - **Favorite / Pinned Tables (`★` / `☆`):** Pin frequently used tables to the top of the Schema Explorer per connection profile.
- **Multi-Tab SQL Editor with 2-Tier Autocomplete:**
  - Custom SQL lexer and syntax highlighter (keywords, functions, types, strings, numbers, comments).
  - **Tier 1 (<1 ms):** Local schema-aware suggestions for tables, `alias.column` completions, Foreign Key `JOIN ... ON` clauses, and SQL keywords.
  - **Tier 2 (AI Ghost Text):** 3 selectable engines in AI Settings:
    - ⚡ **Option 1: Local Schema Native (0ms, 100% Offline):** Instant schema-driven SQL predictions for `SELECT`, `JOIN`, `WHERE`, `ORDER BY`, `INSERT`, `UPDATE` with zero latency.
    - 🦙 **Option 2: Ollama FIM (~100ms):** Ultra-fast local Fill-In-The-Middle inference using `qwen2.5-coder:1.5b`.
    - 🤖 **Option 3: GitHub Copilot:** Native Device Code OAuth flow to connect your official GitHub Copilot subscription.
  - **Quick SQL Macros (`Tab`):** Expand short abbreviations (`sel*`, `selw`, `cnt*`, `join*`, `ins*`, `upd*`, `del*`, `grp*`) directly in the editor.
  - **Parameterized Queries (`:param_name`):** Automatic dynamic parameter bar with safe value substitution before execution.
  - **Persistent Scratchpad Tabs:** Auto-saves open query tabs every 5 seconds and restores them on startup.
  - **Global Command Palette (`Ctrl+K` / `Ctrl+P`):** Instant fuzzy search across tables, saved snippets, and workbench actions.
  - **Professional SQL Formatter (`Ctrl+Shift+F`):** Built-in beautifier (`sqlformat`) that cleans clauses, indentations, and standardizes keyword casing.
  - **Visual Transaction Controls:** Manual transaction mode with `[Begin Tx]`, `[Commit]`, `[Rollback]` buttons, and uncommitted mutation tracking.
- **Interactive Results Grid & 1-Click Data Visualization:**
  - Virtualized multi-column table with column sorting, client-side pagination (25, 50, 100, 250, 500, All), and execution metrics.
  - **Multi-Results Sub-Tabs:** Executing multi-statement SQL scripts renders each result set in its own sub-tab (`Result 1`, `Result 2`, ...).
  - **Developer Copy Formats:** Right-click any column header to copy values as `SQL IN (...)` or `JS/TS Array`.
  - **FK "Browse Related Rows":** Right-click any cell with an outgoing or incoming foreign key relationship to jump directly to the related rows in a new tab.
  - **No-Code Visual Filter Bar:** Instant multi-operator column filtering (`contains`, `=`, `!=`, `starts with`, `>`, `<`, `IS NULL`, `IS NOT NULL`).
  - **Cell Value & JSON Inspector (`🔍`):** Modal inspector with pretty-printed JSON detection, character/byte counters, and copy-to-clipboard.
  - **Inline Cell Editing:** Double-click any cell on a single-table query with a Primary Key to edit values inline and commit parameterized `UPDATE` statements.
  - **1-Click Interactive Charts (`egui_plot`):** Switch instantly between `📋 Table` and `📊 Chart` (Bar and Line charts) with configurable X/Y axis dropdowns.
  - **AI Insights:** One-click `🤖 AI Insights` button to analyze query results and extract key trends, statistics, and anomalies.
  - **One-Click Exports & CSV Import Wizard:** Export results to **CSV**, **JSON**, **Markdown Table**, or **SQL `INSERT`** statements, or import CSV files with column mapping into existing or new tables.
- **Production Guard & Safe Mode (Security):**
  - **Environment Badging:** Tag connections as `Dev` (green), `Staging` (yellow), or `Production` (red).
  - **Read-Only Mode:** Lock any profile to block all mutation queries (`INSERT`, `UPDATE`, `DELETE`, `DROP`, `ALTER`, `TRUNCATE`).
  - **Destructive Query Confirmation Modal:** Intercepts unconstrained `UPDATE` / `DELETE` (without `WHERE`), `DROP`, or `TRUNCATE` before execution.
- **Visual EXPLAIN Plan Tree & Index Advisor:**
  - Color-coded graphical breakdown of execution steps (`ALL` in red, `range` in yellow, `ref/const` in green).
  - **Smart Index Advisor:** Automatically detects full table scans (`ALL`) and filesort bottlenecks, generating recommended `CREATE INDEX` statements with 1-click Copy and Open in Tab.
- **Schema Diff & Synchronization:**
  - Visual comparison between any two databases (e.g. `dev` vs `prod`).
  - Generates executable `ALTER TABLE` and `CREATE TABLE` migration DDL scripts as well as structured Up/Down versioned migration files.
- **Table Designer with Up/Down Migration Generator:**
  - Visual table creation and alteration with one-click generation of reversible migration scripts (`-- >>> UP MIGRATION` / `-- <<< DOWN MIGRATION`).
- **Server Administration & Real-Time Metrics:**
  - Live charts for Queries/sec (QPS) and Active Connections (Threads).
  - Live `SHOW FULL PROCESSLIST` with instant `🛑 Kill Query` action.
  - **Server Variables Inspector:** Filterable real-time browser for `SHOW VARIABLES` with fast search and copy.
- **Snippets & Query Bookmarks:**
  - Save, organize, and load frequent queries with one click; filter history and snippets instantly.
- **Dockable AI SQL Copilot & CLI Assistant:**
  - Automatically injects compact DDL (`TABLE ...`, `FK ...`) of the active database into prompts.
  - Supports **Ollama**, **OpenAI-Compatible APIs**, **Anthropic Claude**, **Google Gemini**, and **Local CLI commands** (`agy`, `llm`, etc.).
  - One-click **`🔧 Fix Query with AI`** button whenever a SQL query returns an error.

---

## 🚀 Building & Running on Ubuntu

### System Prerequisites (Ubuntu / Debian)
```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev libgtk-3-dev \
    libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev \
    libfontconfig1-dev libwayland-dev libx11-dev libgl1-mesa-dev
```

### Run the Application (Development / Direct)
```bash
cargo run --release
```

### Install Globally or User-Wide
You can install the compiled binary directly into your user path:

**Option A: Cargo Install (installs to `~/.cargo/bin/mysql-rusty`)**
```bash
cargo install --path .
```

**Option B: Copy to `~/.local/bin` or `/usr/local/bin`**
```bash
# Build release binary
cargo build --release

# Install for current user:
mkdir -p ~/.local/bin
cp target/release/mysql-rusty ~/.local/bin/

# Or system-wide (requires sudo):
sudo cp target/release/mysql-rusty /usr/local/bin/
```

### Desktop Shortcut (Ubuntu Application Launcher)
To add MySQL-Rusty to your desktop application menu:
```bash
cat << 'EOF' > ~/.local/share/applications/mysql-rusty.desktop
[Desktop Entry]
Name=MySQL Rusty
Comment=Modern Native Rust MySQL Workbench
Exec=mysql-rusty
Icon=utilities-terminal
Terminal=false
Type=Application
Categories=Development;Database;
EOF
```

### Run Unit Tests
```bash
cargo test
```

---

## 🗺 Architecture & Plan
See [`ARCHITECTURE_AND_PLAN.md`](./ARCHITECTURE_AND_PLAN.md) for the detailed architectural blueprint and verified implementation phases.
