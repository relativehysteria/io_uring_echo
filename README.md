# Quick'n'dirty echo server

This code is absolutely fucking terrible. Forks don't propagate anything to the
main thread at all, the server has a manually built async state machine, is 
very far from modular and impossible to use in parallel (i.e. no compute
servers, only i/o bound) and is generally unsound (due to me being reckless and
because the tokio io_uring api just sucks), lacks comments, is written in a 
wacky and hacky and ugly way, doesn't handle DoS (there's no backlog for
operations and no timeouts) and doesn't handle many other errors.
