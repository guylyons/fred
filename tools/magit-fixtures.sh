#!/bin/sh
# Deterministic repositories for the Magit render comparison
# (tools/magit-render.el and src/magit/tests.rs): fixed identities and dates,
# so every commit and stash has the same id on every machine.
#   tools/magit-fixtures.sh DIR    creates DIR/<fixture> for each fixture.
set -eu
root=$1
mkdir -p "$root"
export GIT_AUTHOR_NAME=Ann GIT_AUTHOR_EMAIL=ann@example.test
export GIT_COMMITTER_NAME=Ann GIT_COMMITTER_EMAIL=ann@example.test
export GIT_AUTHOR_DATE='2020-01-02T03:04:05Z' GIT_COMMITTER_DATE='2020-01-02T03:04:05Z'
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 TZ=UTC
g() { git -c init.defaultBranch=main -c core.autocrlf=false "$@"; }

# changes: an upstream, a local commit, untracked, unstaged, staged, a
# rename, a deletion and a stash.
cd "$root" && g init -q upstream && cd upstream
printf 'one\ntwo\nthree\n' > a; printf 'b\n' > b; printf 'gone\n' > d; printf 'old\n' > r
g add . && g commit -qm 'Initial commit'
cd "$root" && g clone -q upstream changes 2>/dev/null && cd changes
printf 'c\n' > c && g add c && g commit -qm 'Add c'
printf 'stashed\n' > b && g stash push -qm 'Saved work'
printf 'one\nTWO\nthree\n' > a
printf 'b2\n' > b && g add b
g mv r renamed && g rm -q d
printf 'new\n' > untracked

# clean: nothing to commit, one commit behind its upstream.
cd "$root" && g clone -q upstream clean 2>/dev/null
cd upstream && printf 'more\n' >> a && g commit -qam 'Upstream change' && cd ../clean && g fetch -q

# conflict: a merge stopped on a conflict.
cd "$root" && g init -q conflict && cd conflict
printf 'base\n' > f && g add f && g commit -qm 'Base'
g checkout -qb topic && printf 'topic\n' > f && g commit -qam 'Topic change'
g checkout -q main && printf 'main\n' > f && g commit -qam 'Main change'
g merge -q topic >/dev/null 2>&1 || true

# rebase: an interactive rebase stopped to edit a commit.
cd "$root" && g init -q rebase && cd rebase
for n in 1 2 3; do printf '%s\n' "$n" > "f$n" && g add "f$n" && g commit -qm "Commit $n"; done
GIT_SEQUENCE_EDITOR="sed -i.bak '1s/^pick/edit/'" g rebase -qi HEAD~2 >/dev/null 2>&1

# detached: HEAD at an older commit; empty: no commits yet.
cd "$root" && g clone -q rebase detached 2>/dev/null && cd detached && g checkout -q HEAD~1
cd "$root" && g init -q empty && printf 'x\n' > empty/x

# modules: a superproject with one populated submodule.
cd "$root" && g init -q modules && cd modules
printf 'top\n' > top && g add top && g commit -qm 'Top'
g -c protocol.file.allow=always submodule add -q ../upstream lib 2>/dev/null
g commit -qm 'Add lib'
