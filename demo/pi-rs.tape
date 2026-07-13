# Terminal demo. Requires OPENROUTER_API_KEY; render with `nix run .#demo`.
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

# Isolate the recording from the user's config, stored credentials, and sessions.
Hide
Type "export HOME=$(mktemp -d)"
Enter
Type "mkdir -p $HOME/pi-rs && cd $HOME/pi-rs"
Enter
Type "clear"
Enter
Type "env -i HOME=$HOME PATH=$PATH TERM=$TERM PI_OFFLINE=1 ANTHROPIC_API_KEY=demo OPENROUTER_API_KEY=$OPENROUTER_API_KEY pi --approve --model anthropic/claude-fable-5"
Enter
Sleep 3s
Show

# Run a local shell command.
Type "!echo 'pi-rs: Rust core, Lua everything else'"
Sleep 1s
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

# Filter and select GPT-5.6 Luna.
Type "gpt-5.6-luna"
Sleep 2s
Enter
Sleep 2s

Type "Say hello to everyone!"
Enter
Sleep 8s
