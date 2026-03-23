#!/bin/bash
# Pong with GPT via NeoRender V2 — AI-to-AI communication
set -e

NEORENDER="$(dirname "$0")/../target/release/neorender"
MESSAGE="${1:-Hola GPT, soy NeoRender V2, un browser engine headless hecho en Rust y V8. Responde con una sola frase corta.}"

echo "[pong] Message: $MESSAGE"
echo "[pong] Starting NeoRender interact..."

# Build the JS that types into ChatGPT's textarea and submits
# ChatGPT uses React — we need nativeInputValueSetter to bypass React's controlled input
TYPE_JS="(function() {
  var ta = document.querySelector('textarea[placeholder=\"Ask anything\"]');
  if (!ta) return 'ERROR: textarea not found';
  var nativeSetter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
  nativeSetter.call(ta, '${MESSAGE}');
  ta.dispatchEvent(new Event('input', {bubbles: true}));
  return 'typed: ' + ta.value.substring(0, 50);
})()"

# Click send — ChatGPT uses data-testid="send-button"
SEND_JS="(function() {
  var btn = document.querySelector('button[data-testid=\"send-button\"]');
  if (!btn) {
    var buttons = document.querySelectorAll('button');
    for (var i = 0; i < buttons.length; i++) {
      if (buttons[i].querySelector('svg') && buttons[i].closest('form')) {
        btn = buttons[i]; break;
      }
    }
  }
  if (!btn) return 'ERROR: send button not found';
  if (btn.disabled) return 'ERROR: send button disabled';
  btn.click();
  return 'clicked send';
})()"

# Read GPT's response — look for assistant message containers
READ_JS="(function() {
  var msgs = document.querySelectorAll('[data-message-author-role=\"assistant\"]');
  if (!msgs.length) {
    msgs = document.querySelectorAll('.markdown, .agent-turn');
  }
  if (!msgs.length) return 'WAITING: no response yet';
  var last = msgs[msgs.length - 1];
  return last.textContent.substring(0, 500);
})()"

{
    # Wait for page to load
    sleep 2

    # Step 1: Type message
    echo "eval $TYPE_JS"
    sleep 1

    # Step 2: Send
    echo "eval $SEND_JS"

    # Step 3: Wait for response (poll every 2s, max 5 tries)
    for i in 1 2 3 4 5; do
        sleep 3
        echo "eval $READ_JS"
    done

    sleep 1
    echo "quit"
} | timeout 60 "$NEORENDER" interact "https://chatgpt.com/" 2>/tmp/pong-gpt-stderr.txt | tee /tmp/pong-gpt-stdout.txt

echo ""
echo "[pong] Done."
