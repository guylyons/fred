#!/bin/sh
# Stand-in for `fred --gui`: one canned frame per input line; exits on "quit".
# Two windows with tab strips (header row 1); the right one is inactive.
frame='{"t":"frame","seq":0,"cols":110,"rows":12,"title":"fake","windows":[
{"x":0,"y":0,"w":55,"h":11,"divider":true,"active":true,"id":1,"header":1,"path":"/tmp/fake.rs",
 "tabs":[{"name":"main.rs","path":"/tmp/main.rs","modified":false,"preview":false,"current":true},{"name":"lib.rs","path":"/tmp/lib.rs","modified":true,"preview":false,"current":false},{"name":"Cargo.toml","path":"/tmp/Cargo.toml","modified":false,"preview":true,"current":false},{"name":"README.md","path":"/tmp/README.md","modified":false,"preview":false,"current":false}],
 "rows":[[{"x":0,"s":"fn ","fg":33515186,"f":1},{"x":3,"s":"main()","fg":4}],[{"x":0,"s":"  hello world"}],[{"x":0,"s":"  日本語 wide","bg":236}]],
 "status":{"mode":"NORMAL","name":"main.rs","modified":false,"pos":"1:1","branch":"main"}},
{"x":55,"y":0,"w":55,"h":11,"divider":false,"active":false,"id":2,"header":1,
 "tabs":[{"name":"notes.md","modified":true,"preview":false,"current":true},{"name":"a_really_long_file_name_that_gets_truncated.rs","modified":false,"preview":false,"current":false}],
 "rows":[[{"x":0,"s":"# notes"}]],
 "status":{"mode":"NORMAL","name":"notes.md","modified":true,"pos":"1:1","branch":"main"}}],
"bottom":[[{"x":0,"s":":"}]],"cursor":{"x":0,"y":1,"shape":"block"}}'
frame=$(printf '%s' "$frame" | tr -d '\n')
while IFS= read -r line; do
  case "$line" in
    *'"quit"'*) echo '{"t":"exit","code":0}'; exit 0 ;;
    *) echo "$frame" ;;
  esac
done
