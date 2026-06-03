See @../../docs/Developer/general/\*

This repo exists to promote the general implementation advice, any code in the other crates is meant to be optimized using utilities that we create here.

This create will only have generics and macros, no implementation, or if there is, it is rather crude.

Currently Contains:

Generics:

1. DataVec, A SoA/AoS Vec, uses comptime information about the underlying type to optimize the way it stores data.
   - It is designed to be a drop-in for all Vecs, However, it doesn't change the access pattern, and still implements the same functions as Vec.
2. DataBTreeMap, ... Same idea as above
3. DataBTreeSet, ... Same idea as above

Macros:

1. #[derive(Data-Optimize)], a derive macro that doesn't change the access pattern, but reasons about the best memory storage pattern according to how it is accessed. Soa vs AoS for each, and auto-upgrades Vec, BTreeMap, BTreeSet -> DataVec, DataBTreeMap, DataBTreeSet if the data storage would be more efficient.
