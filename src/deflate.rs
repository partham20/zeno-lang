//! Makes bytes smaller, the way a `.zip` file does.
//!
//! Every page that `zeno build --web` writes carries `zeno.wasm`, and a
//! page is text, so those bytes are written in base64, which makes them
//! a third longer. To make up for that they are packed first. A browser
//! can unpack them by itself (`DecompressionStream`, in `web.js`), so
//! only the packing is written here.
//!
//! The format is DEFLATE (RFC 1951), which every browser reads. It does
//! two things, one after the other.
//!
//! **Repeats.** Where the bytes say again what they said a little
//! earlier, the repeat is written as "go back this far and copy this
//! many": a *distance* (up to 32768) and a *length* (3 to 258).
//! `zeno.wasm` is full of repeats, because the same few instructions
//! are in it thousands of times. Finding them is the slow part.
//! `Finder` keeps, for every three bytes it has seen, where it last saw
//! them, and from there a chain to where it saw them before that.
//!
//! **Short codes for common things.** What is left is a row of plain
//! bytes, lengths and distances. Each is written in bits, and the ones
//! that come up often get fewer bits (Huffman codes). Which get fewer
//! is worked out from this very data and written in front of it, in a
//! header that is itself packed the same way. `lengths_of` works out
//! how many bits each gets, and `codes_of` gives out the codes.
//!
//! The whole of it is one block. Decision 16.11 says why it was written
//! and not borrowed.

/// Bits are written from the low end of each byte, as DEFLATE says.
struct Bits {
    out: Vec<u8>,
    held: u64,
    count: u32,
}

impl Bits {
    /// Write the low `count` bits of `value`, lowest first.
    fn put(&mut self, value: u32, count: u32) {
        self.held |= (value as u64) << self.count;
        self.count += count;
        while self.count >= 8 {
            self.out.push(self.held as u8);
            self.held >>= 8;
            self.count -= 8;
        }
    }

    /// Write a Huffman code. Those go the other way round: first bit first.
    fn code(&mut self, code: (u16, u8)) {
        let (bits, length) = code;
        self.put((bits.reverse_bits() >> (16 - length as u32)) as u32, length as u32);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.held as u8);
        }
        self.out
    }
}

/// One thing to write: a plain byte, or "copy `length` bytes from
/// `distance` back".
enum Piece {
    Byte(u8),
    Copy { length: u16, distance: u16 },
}

// A length or a distance is written as one of a few dozen symbols, each
// of which stands for a range, and then some extra bits that say where
// in the range. These tables are the start of each range and how many
// extra bits it has, from the RFC.
const LENGTH_FROM: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DISTANCE_FROM: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193,
    12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Which range a length or a distance is in: the last one that starts
/// at or below it.
fn range(from: &[u16], value: u16) -> usize {
    from.partition_point(|start| *start <= value) - 1
}

const WINDOW: usize = 32768;
const LONGEST: usize = 258;
/// How many earlier places are tried for each repeat. More finds longer
/// repeats and takes longer.
const TRIES: usize = 128;
const BUCKETS: usize = 1 << 15;

/// Finds repeats.
struct Finder<'a> {
    data: &'a [u8],
    /// The last place each three bytes were seen (by a hash of them),
    /// plus one. 0 for never.
    head: Vec<u32>,
    /// For each place, the place before it with the same hash, plus one.
    before: Vec<u32>,
}

impl Finder<'_> {
    fn hash(&self, at: usize) -> usize {
        let three = (self.data[at] as u32) << 16 | (self.data[at + 1] as u32) << 8 | self.data[at + 2] as u32;
        (three.wrapping_mul(0x9E37_79B1) >> 17) as usize & (BUCKETS - 1)
    }

    /// Remember that these three bytes are at `at`.
    fn note(&mut self, at: usize) {
        if at + 2 < self.data.len() {
            let hash = self.hash(at);
            self.before[at] = self.head[hash];
            self.head[hash] = at as u32 + 1;
        }
    }

    /// The longest repeat of what is at `at`: how long, and how far back.
    fn longest(&self, at: usize) -> Option<(usize, usize)> {
        if at + 2 >= self.data.len() {
            return None;
        }
        let most = LONGEST.min(self.data.len() - at);
        let mut best = (2, 0);
        let mut seen = self.head[self.hash(at)];
        for _ in 0..TRIES {
            if seen == 0 {
                break;
            }
            let earlier = seen as usize - 1;
            if at - earlier > WINDOW {
                break;
            }
            let same = (0..most).take_while(|n| self.data[earlier + n] == self.data[at + n]).count();
            if same > best.0 {
                best = (same, at - earlier);
                if same == most {
                    break;
                }
            }
            seen = self.before[earlier];
        }
        (best.0 >= 3).then_some(best)
    }
}

/// The bytes as plain bytes and repeats.
fn pieces(data: &[u8]) -> Vec<Piece> {
    let mut finder = Finder { data, head: vec![0; BUCKETS], before: vec![0; data.len()] };
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let found = finder.longest(at);
        finder.note(at);
        match found {
            Some((mut length, mut distance)) => {
                // A longer repeat may start one byte on. If it does,
                // this byte is written plain and that one is taken.
                while let Some((next_length, next_distance)) = finder.longest(at + 1).filter(|next| next.0 > length) {
                    pieces.push(Piece::Byte(data[at]));
                    at += 1;
                    finder.note(at);
                    (length, distance) = (next_length, next_distance);
                }
                pieces.push(Piece::Copy { length: length as u16, distance: distance as u16 });
                for inside in at + 1..at + length {
                    finder.note(inside);
                }
                at += length;
            }
            None => {
                pieces.push(Piece::Byte(data[at]));
                at += 1;
            }
        }
    }
    pieces
}

/// How many bits each symbol's code should have, from how often each
/// comes up: fewer for the common ones, none for one that never does,
/// and never more than `most`.
///
/// This is Huffman's way: put the two rarest together as one thing that
/// is as common as both, again and again, until one thing is left. A
/// symbol's code is as long as the number of times it was put together.
/// Where that makes a code longer than `most`, the counts are halved
/// and it is done again, which flattens the lot.
fn lengths_of(counts: &[u32], most: u8) -> Vec<u8> {
    let mut counts = counts.to_vec();
    loop {
        let mut lengths = vec![0u8; counts.len()];
        // Each group: how common it is, and the symbols in it.
        let mut groups: Vec<(u32, Vec<usize>)> =
            counts.iter().enumerate().filter(|(_, count)| **count > 0).map(|(symbol, count)| (*count, vec![symbol])).collect();
        // A code cannot have no bits, so a lone symbol gets one.
        if groups.len() == 1 {
            lengths[groups[0].1[0]] = 1;
            return lengths;
        }
        while groups.len() > 1 {
            // The rarest go to the end, and the last two are joined.
            groups.sort_by(|a, b| b.0.cmp(&a.0));
            let (a, b) = (groups.pop().expect("two"), groups.pop().expect("two"));
            let mut joined = a.1;
            joined.extend(b.1);
            for symbol in &joined {
                lengths[*symbol] += 1;
            }
            groups.push((a.0 + b.0, joined));
        }
        if lengths.iter().all(|length| *length <= most) {
            return lengths;
        }
        for count in counts.iter_mut().filter(|count| **count > 0) {
            *count = (*count + 1) / 2;
        }
    }
}

/// The code of each symbol, from how many bits each has. DEFLATE fixes
/// which codes those are, so that the lengths alone need be written
/// down: shorter codes come first, and within one length the symbols go
/// in order.
fn codes_of(lengths: &[u8]) -> Vec<(u16, u8)> {
    let mut codes = vec![(0, 0); lengths.len()];
    let mut next = 0u16;
    for length in 1..=15u8 {
        for (symbol, _) in lengths.iter().enumerate().filter(|(_, have)| **have == length) {
            codes[symbol] = (next, length);
            next += 1;
        }
        next <<= 1;
    }
    codes
}

/// The order in which the header writes the lengths of its own codes.
const HEADER_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Pack `data`. What comes back is a raw DEFLATE stream, which
/// `new DecompressionStream("deflate-raw")` unpacks in a browser.
pub fn pack(data: &[u8]) -> Vec<u8> {
    let pieces = pieces(data);

    // How often each symbol comes up. 0 to 255 are the plain bytes, 256
    // ends the block, and 257 on are the ranges of lengths.
    let mut literal_counts = vec![0u32; 286];
    let mut distance_counts = vec![0u32; 30];
    literal_counts[256] = 1;
    for piece in &pieces {
        match piece {
            Piece::Byte(byte) => literal_counts[*byte as usize] += 1,
            Piece::Copy { length, distance } => {
                literal_counts[257 + range(&LENGTH_FROM, *length)] += 1;
                distance_counts[range(&DISTANCE_FROM, *distance)] += 1;
            }
        }
    }
    // A reader wants at least one distance code, even if none is used.
    if distance_counts.iter().all(|count| *count == 0) {
        distance_counts[0] = 1;
    }
    let literal_lengths = lengths_of(&literal_counts, 15);
    let distance_lengths = lengths_of(&distance_counts, 15);
    let (literal_codes, distance_codes) = (codes_of(&literal_lengths), codes_of(&distance_lengths));

    // The header says how long each code is, as one row of numbers: the
    // literals and then the distances. A run of one number is written
    // short: 16 is "the last one, 3 to 6 times more", 17 is "3 to 10
    // zeros", 18 is "11 to 138 zeros".
    let literals = (257..=286).rev().find(|n| literal_lengths[n - 1] > 0).unwrap_or(257);
    let distances = (1..=30).rev().find(|n| distance_lengths[n - 1] > 0).unwrap_or(1);
    let row: Vec<u8> = literal_lengths[..literals].iter().chain(&distance_lengths[..distances]).copied().collect();
    let mut header: Vec<(u8, u8)> = Vec::new(); // the number, and its extra bits
    let mut at = 0;
    while at < row.len() {
        let length = row[at];
        let run = row[at..].iter().take_while(|same| **same == length).count();
        if length == 0 && run >= 3 {
            let run = run.min(138);
            header.push(if run <= 10 { (17, run as u8 - 3) } else { (18, run as u8 - 11) });
            at += run;
        } else if run >= 4 {
            // The length itself once, and then "that again".
            let again = (run - 1).min(6);
            header.push((length, 0));
            header.push((16, again as u8 - 3));
            at += 1 + again;
        } else {
            header.push((length, 0));
            at += 1;
        }
    }
    let mut header_counts = vec![0u32; 19];
    for (number, _) in &header {
        header_counts[*number as usize] += 1;
    }
    let header_lengths = lengths_of(&header_counts, 7);
    let header_codes = codes_of(&header_lengths);
    let written = (4..=19).rev().find(|n| header_lengths[HEADER_ORDER[n - 1]] > 0).unwrap_or(4);

    let mut bits = Bits { out: Vec::with_capacity(data.len() / 2), held: 0, count: 0 };
    bits.put(1, 1); // this is the last block
    bits.put(2, 2); // with codes of its own
    bits.put(literals as u32 - 257, 5);
    bits.put(distances as u32 - 1, 5);
    bits.put(written as u32 - 4, 4);
    for place in &HEADER_ORDER[..written] {
        bits.put(header_lengths[*place] as u32, 3);
    }
    for (number, extra) in &header {
        bits.code(header_codes[*number as usize]);
        match number {
            16 => bits.put(*extra as u32, 2),
            17 => bits.put(*extra as u32, 3),
            18 => bits.put(*extra as u32, 7),
            _ => {}
        }
    }

    for piece in &pieces {
        match piece {
            Piece::Byte(byte) => bits.code(literal_codes[*byte as usize]),
            Piece::Copy { length, distance } => {
                let in_range = range(&LENGTH_FROM, *length);
                bits.code(literal_codes[257 + in_range]);
                bits.put((*length - LENGTH_FROM[in_range]) as u32, LENGTH_EXTRA[in_range] as u32);
                let in_range = range(&DISTANCE_FROM, *distance);
                bits.code(distance_codes[in_range]);
                bits.put((*distance - DISTANCE_FROM[in_range]) as u32, DISTANCE_EXTRA[in_range] as u32);
            }
        }
    }
    bits.code(literal_codes[256]);
    bits.finish()
}

#[cfg(test)]
mod tests {
    use super::pack;
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Unpack with Python's zlib, which is the reader everybody trusts.
    /// `None` on a machine with no Python.
    fn unpacked(packed: &[u8]) -> Option<Vec<u8>> {
        // A machine may have something called `python3` that is not one.
        let works = Command::new("python3").args(["-c", "import zlib; print(1)"]).output().ok()?;
        if !works.status.success() || works.stdout.first() != Some(&b'1') {
            return None;
        }
        let script = "import sys, zlib; sys.stdout.buffer.write(zlib.decompress(sys.stdin.buffer.read(), -15))";
        let mut python = Command::new("python3").args(["-c", script]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
        let mut stdin = python.stdin.take()?;
        let packed = packed.to_vec();
        let writer = std::thread::spawn(move || stdin.write_all(&packed));
        let output = python.wait_with_output().ok()?;
        writer.join().ok()?.ok()?;
        assert!(output.status.success(), "zlib could not read what `pack` wrote");
        Some(output.stdout)
    }

    #[test]
    fn what_is_packed_unpacks_to_the_same() {
        // Bytes that look random, from a small generator.
        let mut seed = 12345u32;
        let noise: Vec<u8> = (0..70_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        let source = include_bytes!("deflate.rs").to_vec();
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"a".to_vec(),
            b"abc".to_vec(),
            vec![0; 100_000],
            b"abcabcabcabcabcabcabcabc abcabc".to_vec(),
            (0..=255).collect(),
            noise,
            source.repeat(3),
        ];
        for case in &cases {
            let packed = pack(case);
            let Some(back) = unpacked(&packed) else { return };
            assert!(back == *case, "{} bytes came back as {} bytes", case.len(), back.len());
        }
        // And it does make things smaller.
        assert!(pack(&source).len() < source.len() / 2);
        assert!(pack(&vec![0; 100_000]).len() < 200);
    }

    /// Not run by `cargo test`: packs the file named in `PACK_FILE` and
    /// says how small it got and how long that took.
    #[test]
    #[ignore]
    fn how_small() {
        let Ok(path) = std::env::var("PACK_FILE") else { return };
        let data = std::fs::read(path).expect("the file");
        let started = std::time::Instant::now();
        let packed = pack(&data);
        println!("{} bytes to {} in {:?}", data.len(), packed.len(), started.elapsed());
        assert!(unpacked(&packed).is_none_or(|back| back == data));
    }
}
