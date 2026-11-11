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
- **Multi-Tab SQL Editor with 2-Tier Autocomplete:**
  - Custom SQL lexer and syntax highlighter (keywords, functions, types, strings, numbers, comments).
  - **Tier 1 (<1 ms):** Local schema-aware suggestions for tables, `alias.column` completions, Foreign Key `JOIN ... ON` clauses, and SQL keywords.
  - **Tier 2 (AI Ghost Text):** Inline Copilot-style ghost completions rendered in muted italic cyan right after the cursor (`Tab` to accept, `Ctrl+Space` to trigger from LLM/CLI).
- **Interactive Results Grid:**
  - Virtualized multi-column table with column sorting, instant client-side row filtering, and execution metrics.
  - **Inline Cell Editing:** Double-click any cell on a single-table query with a Primary Key to edit values inline and commit parameterized `UPDATE` statements.
  - **One-Click Exports:** Export results to **CSV**, **JSON**, or **SQL `INSERT`** statements.
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
