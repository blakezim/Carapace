# Troubleshooting

Common issues and solutions for Carapace.

## Quick Diagnostics

Run these commands to quickly diagnose issues:

```bash
# 1. Is the daemon running?
sudo -u carapace launchctl list | grep carapace

# 2. Does the socket exist?
ls -la /var/run/carapace/

# 3. Are you in the right group?
groups $(whoami) | grep carapace-clients

# 4. Can you connect to the socket?
nc -U /var/run/carapace/gateway.sock

# 5. Check daemon logs
sudo -u carapace tail -50 ~/.local/share/carapace/daemon.log

# 6. Check daemon errors
sudo -u carapace tail -50 ~/.local/share/carapace/daemon.err
```

---

## Connection Issues

### "Connection refused" from shims

**Symptom:**
```
Failed to connect to Carapace gateway: Connection refused
Is the carapace-daemon running?
```

**Causes & Solutions:**

1. **Daemon not running**
   ```bash
   # Check if running
   sudo -u carapace launchctl list | grep carapace

   # Start it
   sudo -u carapace launchctl load ~/Library/LaunchAgents/ai.carapace.gateway.plist
   ```

2. **Socket doesn't exist**
   ```bash
   # Check socket
   ls -la /var/run/carapace/

   # If directory doesn't exist, create it
   sudo mkdir -p /var/run/carapace
   sudo chown carapace:carapace-clients /var/run/carapace
   sudo chmod 750 /var/run/carapace
   ```

3. **Daemon crashed on startup**
   ```bash
   # Check error log
   sudo -u carapace cat ~/.local/share/carapace/daemon.err

   # Common causes:
   # - Invalid config file
   # - Missing binary paths
   # - Permission issues
   ```

### "Permission denied" on socket

**Symptom:**
```
Failed to connect: Permission denied
```

**Causes & Solutions:**

1. **Not in carapace-clients group**
   ```bash
   # Check groups
   groups $(whoami)

   # If missing, add yourself
   sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients

   # LOG OUT AND BACK IN for group to take effect
   ```

2. **Socket permissions wrong**
   ```bash
   # Check permissions
   ls -la /var/run/carapace/gateway.sock

   # Should be: srwxrwx--- carapace carapace-clients
   # If wrong, fix them:
   sudo chown carapace:carapace-clients /var/run/carapace/gateway.sock
   sudo chmod 770 /var/run/carapace/gateway.sock
   ```

3. **Directory permissions wrong**
   ```bash
   # Check directory
   ls -la /var/run/ | grep carapace

   # Should be: drwxr-x--- carapace carapace-clients
   # If wrong:
   sudo chmod 750 /var/run/carapace
   ```

---

## Message Issues

### "Recipient not in allowlist"

**Symptom:**
```
Error: Recipient not in allowlist
```

**Solution:**

1. Check the config:
   ```bash
   sudo -u carapace cat ~/.config/carapace/config.toml | grep -A 10 "outbound"
   ```

2. Add the recipient:
   ```bash
   sudo -u carapace nano ~/.config/carapace/config.toml
   # Add to [channels.imsg.outbound].allowlist
   ```

3. Reload config:
   ```bash
   sudo -u carapace launchctl kickstart -k gui/$(id -u carapace)/ai.carapace.gateway
   ```

### "Rate limit exceeded"

**Symptom:**
```
Error: Rate limit exceeded
```

**Solution:**

1. Wait for the rate limit window to reset (default: 60 seconds)

2. Or increase the limit in config:
   ```toml
   [security.rate_limit]
   imsg = { requests = 60, per_seconds = 60 }  # Double the limit
   ```

### "Content blocked"

**Symptom:**
```
Error: Content blocked: (?i)password
```

**Solution:**

1. Your message matched a content filter pattern
2. Remove sensitive content from the message
3. Or adjust the filter (carefully!):
   ```bash
   sudo -u carapace nano ~/.config/carapace/config.toml
   # Modify [security.content_filter.patterns]
   ```

### Messages not appearing (inbound)

**Symptom:** AI doesn't see incoming messages

**Causes:**

1. **Sender not in inbound allowlist**
   ```bash
   sudo -u carapace cat ~/.config/carapace/config.toml | grep -A 10 "inbound"
   # Add sender to [channels.imsg.inbound].allowlist
   ```

2. **Watch not running**
   ```bash
   # Check if imsg watch is working
   sudo -u carapace /opt/homebrew/bin/imsg watch --json
   # Should show incoming messages
   ```

---

## Daemon Issues

### Daemon won't start

**Check the error log:**
```bash
sudo -u carapace cat ~/.local/share/carapace/daemon.err
```

**Common errors:**

1. **Config parse error**
   ```
   Error: Failed to parse config: expected `=` at line 15
   ```
   Fix: Check TOML syntax in config file

2. **Binary not found**
   ```
   Error: Channel 'imsg' binary not found: /opt/homebrew/bin/imsg
   ```
   Fix: Install imsg or update path in config

3. **Socket already in use**
   ```
   Error: Address already in use
   ```
   Fix: Another daemon is running, or stale socket:
   ```bash
   sudo rm /var/run/carapace/gateway.sock
   ```

### Daemon crashes repeatedly

**Check launchd status:**
```bash
sudo -u carapace launchctl list | grep carapace
# PID of 0 or - means not running
# Check exit status
```

**Check system log:**
```bash
log show --predicate 'subsystem == "com.apple.launchd"' --last 10m | grep carapace
```

**Common causes:**
- Crash in watch loop (check daemon.err)
- Memory issues (rare)
- Channel adapter panic

---

## iMessage Specific Issues

### "Database not found"

**Symptom:**
```
Error: Database not found: /Users/carapace/Library/Messages/chat.db
```

**Solution:**

1. Ensure carapace user is logged into iCloud
2. Open Messages.app as carapace user
3. Wait for sync to complete

### "Full Disk Access required"

**Symptom:** imsg commands fail silently or return empty results

**Solution:**

1. Log in as carapace user
2. System Settings → Privacy & Security → Full Disk Access
3. Add Terminal.app (or the specific app running the daemon)
4. Restart daemon

### Messages not syncing

**Solution:**

1. Log in as carapace user
2. Open System Settings → Apple ID
3. Check iCloud status
4. Open Messages.app and wait for sync
5. Verify with: `imsg chats --limit 5`

---

## Signal Specific Issues

### "Not registered"

**Symptom:**
```
Error: User +1... is not registered
```

**Solution:**

As carapace user:
```bash
signal-cli -a +1YOURPHONE register
signal-cli -a +1YOURPHONE verify CODE
```

### "Captcha required"

**Symptom:** Registration fails with captcha error

**Solution:**

1. Get captcha token from https://signalcaptchas.org/registration/generate.html
2. Register with: `signal-cli -a +1... register --captcha CAPTCHA_TOKEN`

---

## Discord Specific Issues

### "Invalid token"

**Symptom:**
```
Error: Invalid token
```

**Solution:**

1. Regenerate token in Discord Developer Portal
2. Update token file:
   ```bash
   sudo -u carapace nano ~/.config/carapace/discord_token
   ```
3. Restart daemon

### "Missing permissions"

**Symptom:** Bot can't send messages

**Solution:**

1. Check bot permissions in Discord server settings
2. Re-invite bot with correct permissions

---

## Performance Issues

### Slow response times

**Possible causes:**

1. **Content filter regex complexity**
   - Simplify regex patterns
   - Reduce number of patterns

2. **Watch backlog**
   - Messages queuing up
   - Increase `watch_buffer_size` in config

3. **Disk I/O**
   - Audit log on slow disk
   - Consider SSD

### High memory usage

**Check daemon memory:**
```bash
ps aux | grep carapace-daemon
```

**Solutions:**
- Restart daemon periodically
- Reduce `watch_buffer_size`
- Check for message loops

---

## Log Locations

| Log | Path | Contents |
|-----|------|----------|
| Daemon stdout | `~/.local/share/carapace/daemon.log` | Normal operations |
| Daemon stderr | `~/.local/share/carapace/daemon.err` | Errors and crashes |
| Audit log | `~/.local/share/carapace/audit.log` | All requests |
| Dead letters | `~/.local/share/carapace/dead_letters/` | Blocked messages |

All paths are in carapace user's home directory.

---

## Getting Help

If you're still stuck:

1. **Collect diagnostics:**
   ```bash
   # Save to file
   sudo -u carapace cat ~/.local/share/carapace/daemon.log > daemon.log
   sudo -u carapace cat ~/.local/share/carapace/daemon.err > daemon.err
   sudo -u carapace cat ~/.config/carapace/config.toml > config.toml
   # Remove sensitive data from config before sharing!
   ```

2. **Check the issue tracker** (when available)

3. **File an issue** with:
   - macOS version
   - Carapace version
   - Error messages
   - Relevant log excerpts
   - Steps to reproduce
