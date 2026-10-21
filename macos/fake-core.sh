#!/bin/sh
# Stand-in for `fred --gui`: one canned frame per input line; exits on "quit".
# Two windows with tab strips (header row 1); the right one is inactive.
# Both reserve a 13-column minimap margin: the left one maps FAKE_MAP_FILE
# (default src/session.rs) at top 2000; the right one is "too big" (null).
file="${FAKE_MAP_FILE:-$(dirname "$0")/../src/session.rs}"
n=$(wc -l < "$file" | tr -d ' ')
frame='{"t":"frame","seq":0,"cols":120,"rows":36,"title":"fake","windows":[
{"x":0,"y":0,"w":70,"h":35,"divider":true,"active":true,"id":1,"header":1,"margin":13,"top":2000,"lines":'$n',"path":"/tmp/fake.rs",
 "tabs":[{"name":"main.rs","path":"/tmp/main.rs","modified":false,"preview":false,"current":true},{"name":"lib.rs","path":"/tmp/lib.rs","modified":true,"preview":false,"current":false},{"name":"Cargo.toml","path":"/tmp/Cargo.toml","modified":false,"preview":true,"current":false},{"name":"README.md","path":"/tmp/README.md","modified":false,"preview":false,"current":false}],
 "rows":[[{"x":0,"s":"fn ","fg":33515186,"f":1},{"x":3,"s":"main()","fg":4}],[{"x":0,"s":"  hello world"}],[{"x":0,"s":"  日本語 wide","bg":236}]],
 "status":{"mode":"NORMAL","name":"main.rs","modified":false,"pos":"1:1","branch":"main"}},
{"x":70,"y":0,"w":50,"h":35,"divider":false,"active":false,"id":2,"header":1,"margin":13,"top":0,"lines":1,
 "tabs":[{"name":"notes.md","modified":true,"preview":false,"current":true},{"name":"a_really_long_file_name_that_gets_truncated.rs","modified":false,"preview":false,"current":false}],
 "rows":[[{"x":0,"s":"# notes"}]],
 "status":{"mode":"NORMAL","name":"notes.md","modified":true,"pos":"1:1","branch":"main"}}],
"bottom":[[{"x":0,"s":":"}]],"cursor":{"x":0,"y":1,"shape":"block"}}'
frame=$(printf '%s' "$frame" | tr -d '\n')
# Runs of non-blanks per line: comments gray, keywords magenta, strings
# green, Types yellow, calls blue, the rest default.
map=$(awk 'BEGIN { split("fn let pub use impl struct enum match if else return for while mut self Self mod", k, " "); for (i in k) kw[k[i]] = 1 }
{ gsub(/\t/, "    "); line = $0; out = ""; com = line ~ /^[ ]*\/\//; c = 0
  while (match(line, /[^ ]+/)) {
    c += RSTART - 1; t = substr(line, RSTART, RLENGTH)
    col = com ? 8 : (t in kw) ? 5 : t ~ /^"/ ? 2 : t ~ /^[A-Z]/ ? 3 : t ~ /\(/ ? 4 : "null"
    out = out (out == "" ? "" : ",") "[" c "," RLENGTH "," col "]"
    c += RLENGTH; line = substr(line, RSTART + RLENGTH)
  }
  printf "%s[%s]", (NR > 1 ? "," : ""), out }' "$file")
minimap='{"t":"minimap","win":1,"version":1,"lines":['"$map"']}'
while IFS= read -r line; do
  case "$line" in
    *'"quit"'*) echo '{"t":"exit","code":0}'; exit 0 ;;
    *) echo "$frame"; echo "$minimap"; echo '{"t":"minimap","win":2,"version":1,"lines":null}' ;;
  esac
done
