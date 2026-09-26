# Security Policy

## Supported Versions

Security fixes are applied to the current state of the `main` branch. There is
no release published yet, so there is no version table to maintain; once a
version is released this section will list which versions receive fixes.

## Reporting a Vulnerability

**Do not open a public issue for a security vulnerability.**

The preferred channel is GitHub's private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Select **Advisories** → **Report a vulnerability**.
3. Describe the problem using the template below.

Private reports are visible only to the maintainers until an advisory is
published. If the Security tab shows no reporting option, the repository
administrator has not enabled private reporting yet — open an issue that
describes the *category* of the problem and asks the maintainer to enable
private reporting, without including exploit details, proof-of-concept code,
or the affected file paths.

**A public issue is appropriate** for defects that are not sensitive: a crash
on a malformed image, a wrong rendering result, a broken installer path, a
missing keyboard shortcut. Use a public issue for those.

There is **no security contact email address** configured for this project, and
none should be invented or guessed. Use the GitHub Security tab.

## What to Include

- **Version**: the commit SHA you tested, or the release version if one exists.
- **Platform**: Windows version, architecture (x64 or ARM64), GPU, and display
  scale.
- **Reproduction**: the smallest set of steps or files that triggers the
  problem. For an image-parsing or decoding issue, a small test file is far
  more useful than a description; prefer a file under a few hundred KB.
- **Impact**: what an attacker or an ordinary user gains — data loss, memory
  corruption, code execution, information disclosure, or just a crash.
- **Any log output or crash output** verbatim, without reformatting.

## What Happens After You Report

The goal for this project is:

- Acknowledge a well-formed report within a few days.
- Triage it and tell you whether it is accepted as a security issue, treated as
  an ordinary bug, or declined, and why.
- Keep you updated while a fix is prepared, and credit you in the advisory if
  you want credit.

This is a goal for a volunteer-maintained project, not a contractual service
level, and no response time is guaranteed. If a report is not acknowledged, a
follow-up public issue that references the report without restating its
technical details is appropriate.

## Scope

In scope: the application source, the installer script, and the build and
release workflows in this repository.

Out of scope, unless they lead to a defect in the above:

- Vulnerabilities in upstream dependencies with no demonstrated impact in this
  application. Report those upstream; the pinned versions are listed in
  `Cargo.lock` at the repository root.
- Denial of service by opening extremely large or numerous files that the
  application already handles without crashing.
- Missing hardening features. Request those as issues.
