# Task: add `:sort` to Fred

In Vim, `:sort` sorts lines alphabetically, `:5,10sort` sorts only lines 5–10,
and `:sort!` sorts in reverse. Fred doesn't have it yet. It's a good first task
because:

- it all happens in one file, `src/ex/cmd.rs`;
- the command table means adding the command is one new line plus one small
  function;
- there's already a test helper, so you can check your work without opening
  the editor.

**Done when:** `:sort`, `:sort!` and `:3,5sort` work, `:sor` is accepted as a
short form, and Tab completes `sort`.

## 0. Set up

```sh
cd ~/github/fred
git checkout -b sort-command
cargo test --lib ex::cmd        # should pass before you change anything
```

## 1. Read before you write (about 15 minutes)

Open `src/ex/cmd.rs` and find three things.

**The `COMMANDS` table** (search for `pub const COMMANDS`). Each line is one
command. `delete` is the closest model for yours:

```rust
cmd("delete", 1, RANGE, |st, a| {
    st.splice(a.at.start, a.at.end - a.at.start + 1, vec![]);
    ...
```

`|st, a| { ... }` is a closure, Rust's inline function. `st` is the editor
state and `a` holds what the command was given.

**The `Args` struct.** You'll need these fields:

- `a.range` is the range the user typed, if any. Its type is `Option<Range>`,
  meaning it's either `Some(range)` or `None`.
- `a.force` is `true` if they typed `!`.

**Two helpers on `ExState`:**

- `st.lines(range)` returns those lines as a `Vec<String>`, a growable list of
  strings.
- `st.splice(at, remove, insert)` replaces `remove` lines starting at line `at`
  with the `insert` list.

## 2. Write the test first

Find `fn command_table_abbreviations_and_flags` near the bottom of the file and
add a new test after it:

```rust
#[test]
fn sort_lines() {
    assert_eq!(ex("c\na\nb", 0, "sort").0, "a\nb\nc");
    // add: sort!, a range like 2,3sort, and the abbreviation sor
}
```

`ex(text, cursor_line, command)` runs a command on a fake buffer and returns
the new text (`.0`).

Run only your test:

```sh
cargo test --lib sort_lines
```

It should fail with "unknown command". That's the starting point.

## 3. Make it pass

1. **Add a row to `COMMANDS`.** Something like
   `cmd("sort", 3, RANGE | BANG, sort),`. The `3` means `:sor` is the shortest
   form Fred accepts, matching Vim.
2. **Write `fn sort(st: &mut ExState, a: Args) -> Result<ExEffect, String>`**
   next to `join`:
   1. Choose the range. With no range, `:sort` sorts the whole file: copy the
      `all` range from `global_cmd`, then use `a.range.unwrap_or(all)`.
   2. Get the lines with `let mut lines = st.lines(r);`. The `mut` matters,
      because you're about to change the list. Rust variables are read-only
      unless you say `mut`.
   3. Sort them with `lines.sort();`, then `if a.force { lines.reverse(); }`.
   4. Put them back with `st.splice(r.start, ..., lines)`. How many lines are
      you removing?
   5. Return `Ok(ExEffect::None)`.

## 4. Learn to read the compiler's errors

Rust's compiler is strict, but its error messages are the best teacher you'll
get. When `cargo test` fails to compile, read the whole message, especially the
`help:` lines, which often show the exact fix. You'll most likely see:

- **"cannot borrow as mutable"**: you forgot a `mut`.
- **"use of moved value"**: you used a variable after handing it to something
  else. Usually you can reorder the lines or add `.clone()`.
- **"mismatched types … expected `Result`"**: you forgot to wrap the return
  value in `Ok(...)`.

## 5. Before you call it done

```sh
cargo fmt                        # auto-formats your code
cargo clippy --all-targets       # lint; fix anything new it points at
cargo test                       # the whole suite
cargo install --path .           # then try :sort in real Fred
```

One test, `bisect_finds_the_bad_commit_and_runs_scripts`, already fails on
`main`, so ignore it.

## Stretch goals, if it goes well

- `:sort u` removes duplicate lines (look up `Vec::dedup`).
- `:sort i` ignores case (look up `sort_by_key` and `to_lowercase`;
  `src/complete/mod.rs` uses both).
- Then remove `BANG` from your table row and run the test again to see the
  `no ! allowed` check work.

If you get stuck, paste the compiler error to Claude and ask for an
explanation rather than a fix. When it works, ask for a review of the diff.
