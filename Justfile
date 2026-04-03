tui-dev:
	watchexec -w crates -w src -e rs,toml,json --restart -- cargo run -p ironclaw -- tui
