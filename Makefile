# nba3k — common run / build / verify targets.
#
# All targets call `cargo` directly with the toolchain pinned in
# rust-toolchain.toml. See docs/RUNNING.md and docs/VERIFICATION.md.

.PHONY: help build release test test-ignored lint fmt fmt-check verify clean clean-cache new live tui scripted-season seed

CARGO := cargo
BIN := nba3k
EXAMPLE_SAVE := /tmp/nba3k_dev.db

help:
	@echo "nba3k — common targets"
	@echo ""
	@echo "  build           debug build, all crates"
	@echo "  release         release build, just the user binary"
	@echo "  test            cargo test --workspace"
	@echo "  test-ignored    run #[ignore]'d tests too (needs release bin + seed)"
	@echo "  lint            clippy + rustfmt --check"
	@echo "  fmt             apply rustfmt"
	@echo "  fmt-check       rustfmt --check (CI parity)"
	@echo "  verify          build + test + lint (run before commit)"
	@echo "  clean           remove target/"
	@echo "  clean-cache     remove data/cache/espn/ (force re-fetch)"
	@echo ""
	@echo "  new             create $(EXAMPLE_SAVE) via live ESPN import (default)"
	@echo "  live            same as 'new' (alias)"
	@echo "  tui             open $(EXAMPLE_SAVE) in TUI"
	@echo "  scripted-season replay tests/scripts/season1.txt against a fresh save"
	@echo "  seed            rebuild data/seed_2025_26.sqlite (slow, ~3 min)"

# ---- build ----------------------------------------------------------------

build:
	$(CARGO) build --workspace

release:
	$(CARGO) build --release --bin $(BIN)

# ---- verify ---------------------------------------------------------------

test:
	$(CARGO) test --workspace

test-ignored:
	$(CARGO) test --workspace -- --ignored

lint:
	$(CARGO) clippy --workspace --all-targets
	$(CARGO) fmt --all -- --check

# Strict lint — treats every warning as an error. Currently fails on a
# handful of legacy nits in nba3k-trade / nba3k-cli (tracked in
# docs/PROGRESS.md). Aspirational target; plain `lint` is the gate.
lint-strict:
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) fmt --all -- --check

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

verify: build test lint

# ---- run shortcuts (use $(EXAMPLE_SAVE)) ----------------------------------

new live:
	rm -f $(EXAMPLE_SAVE) $(EXAMPLE_SAVE)-shm $(EXAMPLE_SAVE)-wal
	$(CARGO) run --release --bin $(BIN) -- \
	  --save $(EXAMPLE_SAVE) new --team BOS

new-offline:
	rm -f $(EXAMPLE_SAVE) $(EXAMPLE_SAVE)-shm $(EXAMPLE_SAVE)-wal
	$(CARGO) run --release --bin $(BIN) -- \
	  --save $(EXAMPLE_SAVE) new --team BOS --offline

tui:
	$(CARGO) run --release --bin $(BIN) -- --save $(EXAMPLE_SAVE) tui

scripted-season:
	rm -f /tmp/nba3k_replay.db /tmp/nba3k_replay.db-shm /tmp/nba3k_replay.db-wal
	$(CARGO) build --release --bin $(BIN)
	./target/release/$(BIN) --save /tmp/nba3k_replay.db new --team BOS --offline
	./target/release/$(BIN) --save /tmp/nba3k_replay.db --script tests/scripts/season1.txt

seed:
	$(CARGO) run -p nba3k-scrape --release -- --out data/seed_2025_26.sqlite

# ---- cleanup --------------------------------------------------------------

clean:
	$(CARGO) clean

clean-cache:
	rm -rf data/cache/espn
