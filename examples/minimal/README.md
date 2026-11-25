# Minimal external project

This project demonstrates a real Docker runner without additional Python dependencies. Its behavior contract is: `bounded_value` returns a value between zero and ten, inclusive. The implementation deliberately omits the lower bound so a regression test can demonstrate a defect without changing production code.

Copy this directory to a separate Git repository with a remote `main`, then run `aaqe` from that repository. The CLI creates its audit branch from remote main, so the example must be committed and pushed there first. Environment and test definitions are project-side input; AAQE contains no knowledge of this example's behavior.
