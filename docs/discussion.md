# Flat Domain vs Versioned Domain

How we implement the physical domain.
More important than local vs remote.

## Flat Domain

We already got two requests for this feature on S3

# Local Domain vs Remote (S3) Domain

How we synchronize the logical domains.
This is what is used today.

# Logical Domain vs Physical Domain

Fundamental question: which Rust classes to I/O.
Does every element in the logical hierarchy need to do I/O?

# Who handles I/O?

What are the differences in how Rust handles I/O vs Python?
Python has a upath abstraction for every logical object.
It abstracts away the fact whether it's local or remote.

Is I/O a method, a module, or a trait?

# how to create a new version of a package

# Operations in the system
