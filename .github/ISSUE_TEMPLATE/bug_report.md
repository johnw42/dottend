---
name: Bug report
about: Create a report to help us improve
title: "[BUG] "
labels: ''
assignees: ''

---

## Environment
- **OS**: Linux distribution / Windows version
- **Dottend version**: Find out using `dottend -V`
- **Additional relevant information**

## Description
A clear and concise description of what the bug is.

## Reproduction
Steps to reproduce the issue. Include your configuration and the exact command that needs to be executed.

### Expected behavior
A clear and concise description of what you expected to happen.

### Actual behavior
What actually happened.
Include an error message if applicable, preferably using the `RUST_LOG=trace` environment variable like so:
```
export RUST_LOG=trace  # linux
set RUST_LOG=trace  # windows

dottend  # Should now give a lot more output
```
