#!/bin/sh
# Stand-in for `fred --gui`: one canned frame per input line; exits on "quit".
frame='{"t":"frame","seq":0,"cols":40,"rows":8,"title":"fake","windows":[{"x":0,"y":0,"w":40,"h":7,"divider":false,"active":true,"rows":[[{"x":0,"s":"fn ","fg":33515186,"f":1},{"x":3,"s":"main()","fg":4}],[{"x":0,"s":"  hello world"}],[{"x":0,"s":"  日本語 wide","bg":236}]],"status":{"mode":"NORMAL","name":"fake.rs","modified":false,"pos":"1:1","branch":"main"}}],"bottom":[[{"x":0,"s":":"}]],"cursor":{"x":0,"y":0,"shape":"block"}}'
while IFS= read -r line; do
  case "$line" in
    *'"quit"'*) echo '{"t":"exit","code":0}'; exit 0 ;;
    *) echo "$frame" ;;
  esac
done
