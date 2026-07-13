# Reproducible, offline terminal demo. Render with `nix run .#demo`.
Output "demo/pi-rs.gif"

Require pi

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 720
Set Padding 24
Set Framerate 30
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"
Set WindowBar "ColorfulRight"
Set WindowBarSize 32

# Isolate the recording from the user's config, credentials, and sessions.
Hide
Type "export HOME=$(mktemp -d)"
Enter
Type "mkdir -p $HOME/pi-rs && cd $HOME/pi-rs"
Enter
Type "export ANTHROPIC_API_KEY=demo"
Enter
Type "export PI_OFFLINE=1"
Enter
Type "clear"
Enter
Show

Type "pi --approve --model anthropic/claude-sonnet-4-5"
Enter
Sleep 3s

# Show slash-command completion, then open the model picker.
Type "/"
Sleep 2s
Type "model"
Sleep 1s
Escape
Enter
Sleep 2s

# Filter and select a different model without making a provider request.
Type "opus-4-8"
Sleep 2s
Enter
Sleep 2s

# Show the built-in keyboard reference.
Type "/hotkeys"
Sleep 1s
Escape
Enter
Sleep 3s

# Run a local shell command; this demo never contacts a model provider.
Type "!echo 'pi-rs: Rust core, Lua everything else'"
Sleep 1s
Enter
Sleep 3s

Type "/quit"
Sleep 1s
Escape
Enter
Sleep 2s
