# Security

If you find a security problem in Zeno, please report it in private
first, so that it can be fixed before everyone can read about it.

## How to report

Write to **security@zeno-lang.org**.

Please do not open a public issue or a pull request for it: both show
the problem to everybody before there is a fix.

Say as much of this as you can:

- what the problem is, and what somebody could do with it
- the version (`zeno --version`) and the system it ran on
- the smallest program, package or steps that show it

## What counts

- `zeno` and its tools (`zeno run`, `zeno build`, `zeno fetch`,
  `zeno format`, `zeno lsp`) doing what a program or a package should
  not be able to make them do. For example: touching files nobody
  asked for, or running code when the command was only to check or
  format it.
- The installers (`install.sh` and the Windows setup program) and the
  packages of a release.
- The website, [zeno-lang.org](https://zeno-lang.org), and its
  playground.

A program that crashes, or a wrong answer from the compiler, is an
ordinary bug. Those are welcome as
[issues](https://github.com/partham20/zeno-lang/issues), or at
issues@zeno-lang.org if you have no GitHub account.

## Which versions

A fix goes into the newest release. Zeno is before 1.0, so older
versions are not mended: install the newest one.

## What happens next

Zeno is made by one person in his spare time, so there is no promised
time for an answer. Every report is read and answered. The fix comes
out in a new release, and its notes thank you by name unless you would
rather they did not.

## Everything else

For help with installing or using Zeno, write to help@zeno-lang.org.
For anything else, write to contact@zeno-lang.org.
