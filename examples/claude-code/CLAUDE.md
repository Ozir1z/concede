## Decision log (Concede)

This repository keeps a decision log in `.concede/`. Your edits are captured
automatically, but only you can say why you made them.

- After each logical change (one intent, possibly several files), call the
  `record_decision` tool with the task, your reasoning, the files you changed and
  any alternatives you rejected. Pass your model id as `model`.
- Before changing code you didn't write in this session, call `why` on the file or
  lines to see earlier decisions and their reasoning. Don't silently undo a decision
  that was approved; mention it instead.
- Never edit or delete files in `.concede/` yourself.
