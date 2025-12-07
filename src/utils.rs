pub fn simple_similarity(a: &str, b: &str) -> f32 {
    let ab = a.as_bytes();
    let bb = b.as_bytes();

    let len_a = ab.len();
    let len_b = bb.len();
    let min_len = len_a.min(len_b);

    let mut score = 0;

    for i in 0..min_len {
        if ab[len_a - 1 - i] == bb[len_b - 1 - i] {
            score += 1;
        }
    }

    score as f32 / min_len as f32
}
