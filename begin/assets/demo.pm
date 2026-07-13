sheet demo {
    cell a: f64 = 2.0;
    cell b: f64 = 3.0;
    cell c: f64;
    cell d: f64 = 4.0;
    cell e: f64 = 5.0;
    cell f: f64;
    cell g = 0.0;
    cell p: i32 = 0;

    relationship {
        method [a, b] -> [c] { a * b }
        method [b, c] -> [a] { c / b }
        method [a, c] -> [b] { c / a }
    }

    relationship {
        method [d, e] -> [f] { d * e }
        method [e, f] -> [d] { f / e }
        method [d, f] -> [e] { f / d }
    }

    conditional p {
        0i32 => {
            method [f] -> [c] { f }
            method [c] -> [f] { c }
        }
        1i32 => {
            method [f] -> [c] { f * 2.0 }
            method [c] -> [f] { c / 2.0 }
        }
    }

    conditional p {
        1i32 => {
            method [c] -> [g] { c * 10.0 }
        }
    }
}
