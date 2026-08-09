# Baukit CLI

Install the repository checkout's pinned CLI on `PATH`:

```sh
make --directory cli install
baukit --version
```

The equivalent direct command is `cargo install --path cli --locked`. Re-run it
after checking out a different Baukit release so the CLI's embedded templates
and release dependency tag stay aligned.

`baukit new NAME ...` creates a new `NAME/` directory. To scaffold an existing
or orphan-branch repository root without overwriting differing files, use
`--dir . --into-existing`. Generation deliberately does not commit or push:

```sh
git add .
git commit -m "Scaffold product with Baukit"
git remote add origin git@github.com:YOUR_ORG/NAME.git
git push -u origin main
```
