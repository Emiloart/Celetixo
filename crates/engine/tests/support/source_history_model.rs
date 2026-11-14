//! Independent finite source-history interpreter shared only by test harnesses.
//! Numeric vertices and a reachability matrix avoid production access/analyzer code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct History {
    pub flags: u16,
}
impl History {
    pub fn generated(seed: u16) -> Self {
        // The first cases pin empty, each singleton, both edges and all features.
        const CASES: [u16; 12] = [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 6, 511];
        Self {
            flags: CASES
                .get(seed as usize)
                .copied()
                .unwrap_or_else(|| seed.wrapping_mul(157).wrapping_add(29) & 511),
        }
    }
    pub fn order(self) -> [usize; 2] {
        if self.flags & 1 == 0 { [0, 1] } else { [1, 0] }
    }
    pub fn edge(self, consumer: usize, dependency: usize) -> bool {
        consumer != dependency && self.flags & (1 << (1 + consumer)) != 0
    }
    pub fn explicit_read(self, transaction: usize) -> bool {
        self.flags & (1 << (3 + transaction)) != 0
    }
    pub fn scope(self, transaction: usize) -> bool {
        self.flags & (1 << (5 + transaction)) != 0
    }
    pub fn predicate(self, transaction: usize) -> bool {
        self.flags & (1 << (7 + transaction)) != 0
    }
    pub fn reads(self, transaction: usize) -> [bool; 2] {
        let mut reaches = [[false; 2]; 2];
        for (consumer, dependencies) in reaches.iter_mut().enumerate() {
            for (dependency, slot) in dependencies.iter_mut().enumerate() {
                *slot = self.edge(consumer, dependency);
            }
        }
        for middle in 0..2 {
            for consumer in 0..2 {
                for dependency in 0..2 {
                    reaches[consumer][dependency] |=
                        reaches[consumer][middle] && reaches[middle][dependency];
                }
            }
        }
        let other = 1 - transaction;
        // A declared invariant scope participates in conflicts and dependency
        // expansion. It is not itself a value read without an invariant contract.
        let observes_other = self.explicit_read(transaction) || self.predicate(transaction);
        let mut roots = [false; 2];
        roots[transaction] = true;
        roots[other] = observes_other || self.scope(transaction);
        let mut reads = [false; 2];
        reads[other] = observes_other;
        for consumer in 0..2 {
            if reaches[consumer][transaction] {
                roots[consumer] = true;
                reads[consumer] = true;
            }
        }
        for root in 0..2 {
            if roots[root] {
                for (dependency, slot) in reads.iter_mut().enumerate() {
                    *slot |= reaches[root][dependency];
                }
            }
        }
        reads
    }
    pub fn original_conflict(self) -> bool {
        self.reads(0)[1]
            || self.reads(1)[0]
            || self.scope(0)
            || self.scope(1)
            || self.predicate(0)
            || self.predicate(1)
    }
    pub fn output(self, transaction: usize, values: [u64; 2]) -> u64 {
        values[transaction]
            + self
                .reads(transaction)
                .iter()
                .enumerate()
                .filter_map(|(i, read)| read.then_some(values[i]))
                .sum::<u64>()
            + 7
            + transaction as u64
    }
    pub fn serial(self) -> [u64; 2] {
        let mut values = [1, 2];
        for transaction in self.order() {
            values[transaction] = self.output(transaction, values);
        }
        values
    }
    pub fn reductions(self) -> Vec<Self> {
        (0..9)
            .filter(|bit| self.flags & (1 << bit) != 0)
            .map(|bit| Self {
                flags: self.flags & !(1 << bit),
            })
            .collect()
    }
}
/// Every retained reduction is actually replayed by the caller's mismatch check.
pub fn minimize(mut history: History, mut mismatch: impl FnMut(History) -> bool) -> History {
    assert!(mismatch(history));
    while let Some(smaller) = history
        .reductions()
        .into_iter()
        .find(|candidate| mismatch(*candidate))
    {
        history = smaller;
    }
    history
}
