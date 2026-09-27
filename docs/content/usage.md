# Usage and CLI

The main commands are:

```text
jiotv login otp
jiotv admin password
jiotv serve [--host HOST] [--port PORT]
jiotv epg generate|delete
jiotv autostart [--args "serve flags"]
jiotv update [--version VERSION]
```

Run `jiotv --help` for the options supported by the installed build. Keep the data directory private; it stores authentication material. Use `jiotv serve` to run in the foreground, or configure the platform's service manager as described in its installation guide.
