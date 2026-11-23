// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

// Debug tool: feed a raw byte capture into the VT and print the grid.
use tuxixi::vt::Terminal;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: vtdump <capture.bin>");
    let bytes = std::fs::read(&path).expect("read capture");
    let mut term = Terminal::new(100, 24);
    term.feed(&bytes);
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let cols = term.cols as usize;
    for y in 0..term.rows {
        let start = y as usize * cols;
        let line: String = grid[start..start + cols].iter().map(|c| c.c).collect();
        println!("{:02}|{}", y, line.trim_end());
    }
}
