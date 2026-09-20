# 0008. Compilation is partial

- Status: Accepted
- Date: 2026-09-20

## Context

Loading a model means compiling every field and metric expression. Some of them
will fail — an expression using a construct this crate does not support, a
reference to a field that does not exist, a typo.

If loading is all-or-nothing, one unsupported metric makes the entire model
unusable, including every metric that compiles perfectly. Real models are not
uniformly within reach of any one tool, and refusing all of a model because of
one part of it is a bad trade: the caller loses eight working metrics to protect
them from one broken one.

But not every problem is local. Two datasets with the same name make every
lookup ambiguous; a relationship naming a dataset that does not exist breaks
join planning. There is no sensible partial answer to those.

## Decision

Split by whether the problem is local.

- **Structural problems are fatal.** Wrong spec version, duplicate dataset,
  field or metric names, relationships referencing datasets that do not exist,
  mismatched key column counts. These leave the model ambiguous, so loading
  fails.
- **Expression problems are not.** A field or metric whose expression will not
  compile is recorded as an issue and left out of the usable set. The rest of
  the model loads. Referencing the excluded item reports the original reason
  rather than claiming it does not exist.

A strict entry point is available for callers who want the whole model or
nothing.

## Consequences

- One unsupported metric costs you that metric.
- Every expression problem in a model is reported together at load, rather than
  one at a time as queries happen to touch them.
- A caller that ignores the issue list may be surprised later by a metric that
  is missing rather than broken. The error at that point names the reason.

