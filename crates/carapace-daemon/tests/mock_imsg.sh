#!/bin/bash
# Mock imsg binary for integration tests.
# Mimics the real `imsg` CLI interface with canned responses.

set -e

CMD="${1:-}"

case "$CMD" in
    send)
        echo "Message sent successfully"
        exit 0
        ;;
    chats)
        if [[ " $* " == *" --json "* ]]; then
            cat <<'CHATS'
[{"chat_id":"chat001","display_name":"Alice","service_name":"iMessage"},{"chat_id":"chat002","display_name":"Bob","service_name":"iMessage"}]
CHATS
        else
            echo "chat001: Alice"
            echo "chat002: Bob"
        fi
        exit 0
        ;;
    history)
        if [[ " $* " == *" --json "* ]]; then
            cat <<'HISTORY'
[{"sender":"+1111111111","text":"Hey there","date":"2025-01-01T00:00:00Z"},{"sender":"+2222222222","text":"Hello!","date":"2025-01-01T00:01:00Z"}]
HISTORY
        else
            echo "[+1111111111]: Hey there"
            echo "[+2222222222]: Hello!"
        fi
        exit 0
        ;;
    watch)
        if [[ " $* " == *" --json "* ]]; then
            # Emit 3 events: 2 from allowlisted senders, 1 from a non-allowlisted sender.
            sleep 0.1
            echo '{"sender":"+1111111111","text":"hello from allowed","date":"2025-01-01T00:00:00Z"}'
            sleep 0.1
            echo '{"sender":"+9999999999","text":"hello from blocked","date":"2025-01-01T00:00:01Z"}'
            sleep 0.1
            echo '{"sender":"+1111111111","text":"second from allowed","date":"2025-01-01T00:00:02Z"}'
            # Then exit (stream ends).
        fi
        exit 0
        ;;
    *)
        echo "Unknown command: $CMD" >&2
        exit 1
        ;;
esac
