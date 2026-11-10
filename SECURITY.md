# Security reporting

Do not put private records, account identifiers, credentials, statements, backups,
or raw logs into public issues or PR artifacts. Reproduce with synthetic inputs.
Use GitHub's private vulnerability reporting for shaug/currents if enabled. If it
is unavailable, open a public issue containing only a request for a private contact
channel, with no vulnerability details or private attachments. Never assume a
private reporting feature or protected branch has been enabled by this repository.

Privacy gates use pinned Gitleaks default rules plus an account-marker canary,
100% redaction and suppressed scanner diagnostics on failure. CI uploads no raw
failure artifacts. A scan is defense in depth, not proof arbitrary financial data
is safe to publish. Only synthetic material belongs in this repository.
Signing and institutional connections need separate authorization and setup.
