# Persistent project task notes

When working on the Magit port, read [docs/magit-port-notes.md](docs/magit-port-notes.md)
and [docs/magit-parity.md](docs/magit-parity.md) first. Follow the percentage-based
[completion roadmap](docs/magit-roadmap.md), updating accepted points and the
execution queue in every implementation commit. The user's binding goal is
full parity with https://github.com/magit/magit: menus, options, functionality
and workflow behavior, adapted from Emacs to Fred's Rust component. Global entry
keys live under `Space m`. A Magit-inspired subset or simple Git wrappers do not
satisfy the task. Use the pinned upstream source as the behavioral reference,
track every incomplete feature explicitly, and continue implementation without
repeated permission questions. Preserve the user's uncommitted `notes.md`.
