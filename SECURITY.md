# Security Policy

## Supported versions

Only the latest release gets security fixes.

## Reporting a vulnerability

Report privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Please do not open a public issue.

Expect a first reply within 7 days. Fixes ship in a new release.

## Verifying a download

Mouthy is not signed or notarised by Apple, so check the dmg against the `SHA256SUMS` file attached to the same release before opening it:

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
```
