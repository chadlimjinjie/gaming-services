//! Pure Sudoku rules and puzzle generation. This is the authoritative source of
//! truth; the TypeScript in `apps/web-beta/features/sudoku/game-logic.ts` mirrors
//! only the bits needed for instant client-side feedback (conflict hints).
//!
//! A `Grid` is a 9x9 array of digits where `0` means empty. A `Puzzle` bundles a
//! full `solution` with the `givens` (clue) grid handed to both players.

// Row/column index arithmetic over the fixed 9x9 grid reads more clearly with
// explicit indices than with zipped iterators (and several loops index two axes
// with one variable), so the range-loop lint is not helpful here.
#![allow(clippy::needless_range_loop)]

use rand::seq::SliceRandom;
use rand::Rng;

pub const SIZE: usize = 9;
pub const BOX: usize = 3;

/// A 9x9 board. `0` is an empty cell; `1..=9` are filled digits.
pub type Grid = [[u8; SIZE]; SIZE];

/// A 9x9 boolean mask (e.g. which cells a player has filled).
pub type Mask = [[bool; SIZE]; SIZE];

/// Difficulty controls how many cells are dug out of the full solution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    /// Parse the wire string, defaulting to Medium for anything unknown.
    pub fn parse(s: &str) -> Difficulty {
        match s {
            "easy" => Difficulty::Easy,
            "hard" => Difficulty::Hard,
            _ => Difficulty::Medium,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
        }
    }

    /// Number of cells to remove from the full grid. More holes = harder.
    fn holes(self) -> usize {
        match self {
            Difficulty::Easy => 40,
            Difficulty::Medium => 50,
            Difficulty::Hard => 56,
        }
    }
}

/// A generated puzzle: the unique `solution` and the `givens` shown to players.
#[derive(Clone, Debug)]
pub struct Puzzle {
    pub solution: Grid,
    pub givens: Grid,
}

/// True if placing `d` at (row, col) keeps the row, column, and 3x3 box valid.
pub fn is_valid(grid: &Grid, row: usize, col: usize, d: u8) -> bool {
    for i in 0..SIZE {
        if grid[row][i] == d || grid[i][col] == d {
            return false;
        }
    }
    let (br, bc) = (row / BOX * BOX, col / BOX * BOX);
    for r in br..br + BOX {
        for c in bc..bc + BOX {
            if grid[r][c] == d {
                return false;
            }
        }
    }
    true
}

/// Fill an empty grid with a random, valid, complete solution via randomized
/// backtracking. Always succeeds (a valid completion always exists from empty).
pub fn generate_full<R: Rng>(rng: &mut R) -> Grid {
    let mut grid = [[0u8; SIZE]; SIZE];
    fill(&mut grid, 0, rng);
    grid
}

fn fill<R: Rng>(grid: &mut Grid, pos: usize, rng: &mut R) -> bool {
    if pos == SIZE * SIZE {
        return true;
    }
    let (row, col) = (pos / SIZE, pos % SIZE);
    let mut digits: [u8; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    digits.shuffle(rng);
    for &d in &digits {
        if is_valid(grid, row, col, d) {
            grid[row][col] = d;
            if fill(grid, pos + 1, rng) {
                return true;
            }
            grid[row][col] = 0;
        }
    }
    false
}

/// Count solutions of `grid`, stopping early once `limit` is reached. Pass
/// `limit = 2` to cheaply test whether a puzzle has a *unique* solution.
pub fn count_solutions(grid: &Grid, limit: usize) -> usize {
    let mut work = *grid;
    let mut count = 0;
    solve(&mut work, &mut count, limit);
    count
}

fn solve(grid: &mut Grid, count: &mut usize, limit: usize) {
    if *count >= limit {
        return;
    }
    // Find the first empty cell.
    let mut spot = None;
    'outer: for r in 0..SIZE {
        for c in 0..SIZE {
            if grid[r][c] == 0 {
                spot = Some((r, c));
                break 'outer;
            }
        }
    }
    let (row, col) = match spot {
        Some(rc) => rc,
        None => {
            *count += 1;
            return;
        }
    };
    for d in 1..=9u8 {
        if is_valid(grid, row, col, d) {
            grid[row][col] = d;
            solve(grid, count, limit);
            grid[row][col] = 0;
            if *count >= limit {
                return;
            }
        }
    }
}

/// Remove up to `holes` cells from `full`, keeping a removal only when the puzzle
/// still has exactly one solution. Returns the resulting givens grid.
pub fn dig_holes<R: Rng>(full: &Grid, holes: usize, rng: &mut R) -> Grid {
    let mut givens = *full;
    let mut positions: Vec<usize> = (0..SIZE * SIZE).collect();
    positions.shuffle(rng);

    let mut removed = 0;
    for pos in positions {
        if removed >= holes {
            break;
        }
        let (r, c) = (pos / SIZE, pos % SIZE);
        let backup = givens[r][c];
        if backup == 0 {
            continue;
        }
        givens[r][c] = 0;
        if count_solutions(&givens, 2) == 1 {
            removed += 1;
        } else {
            givens[r][c] = backup; // removal broke uniqueness — put it back
        }
    }
    givens
}

/// Generate a puzzle for the given difficulty: a full solution plus a unique
/// givens grid dug to that difficulty's hole count.
pub fn generate<R: Rng>(difficulty: Difficulty, rng: &mut R) -> Puzzle {
    let solution = generate_full(rng);
    let givens = dig_holes(&solution, difficulty.holes(), rng);
    Puzzle { solution, givens }
}

/// Number of cells (0..=81) whose value matches the solution.
pub fn correct_count(board: &Grid, solution: &Grid) -> u8 {
    let mut n = 0u8;
    for r in 0..SIZE {
        for c in 0..SIZE {
            if board[r][c] == solution[r][c] {
                n += 1;
            }
        }
    }
    n
}

/// True when every cell matches the solution.
pub fn is_complete_correct(board: &Grid, solution: &Grid) -> bool {
    correct_count(board, solution) == (SIZE * SIZE) as u8
}

/// Which cells are non-empty. Broadcast to the opponent instead of the digits.
pub fn filled_mask(board: &Grid) -> Mask {
    let mut mask = [[false; SIZE]; SIZE];
    for r in 0..SIZE {
        for c in 0..SIZE {
            mask[r][c] = board[r][c] != 0;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn is_complete_valid(grid: &Grid) -> bool {
        for r in 0..SIZE {
            for c in 0..SIZE {
                let d = grid[r][c];
                if d == 0 {
                    return false;
                }
                // Temporarily clear to reuse is_valid for a duplicate check.
                let mut g = *grid;
                g[r][c] = 0;
                if !is_valid(&g, r, c, d) {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn full_grid_is_valid_and_unique() {
        let mut rng = StdRng::seed_from_u64(42);
        let grid = generate_full(&mut rng);
        assert!(is_complete_valid(&grid), "full grid must be a valid sudoku");
        assert_eq!(count_solutions(&grid, 2), 1, "a full grid has one solution");
    }

    #[test]
    fn generated_puzzle_has_unique_solution() {
        let mut rng = StdRng::seed_from_u64(7);
        for diff in [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard] {
            let p = generate(diff, &mut rng);
            assert_eq!(
                count_solutions(&p.givens, 2),
                1,
                "{:?} puzzle must have exactly one solution",
                diff
            );
        }
    }

    #[test]
    fn givens_are_subset_of_solution() {
        let mut rng = StdRng::seed_from_u64(99);
        let p = generate(Difficulty::Medium, &mut rng);
        for r in 0..SIZE {
            for c in 0..SIZE {
                let g = p.givens[r][c];
                if g != 0 {
                    assert_eq!(g, p.solution[r][c], "given must match the solution");
                }
            }
        }
    }

    #[test]
    fn correct_count_and_completion() {
        let mut rng = StdRng::seed_from_u64(1);
        let p = generate(Difficulty::Easy, &mut rng);
        assert_eq!(correct_count(&p.solution, &p.solution), 81);
        assert!(is_complete_correct(&p.solution, &p.solution));
        // The givens grid (with holes) is not complete.
        assert!(!is_complete_correct(&p.givens, &p.solution));
        assert!(correct_count(&p.givens, &p.solution) < 81);
    }

    #[test]
    fn difficulty_hole_counts_increase() {
        assert!(Difficulty::Easy.holes() < Difficulty::Medium.holes());
        assert!(Difficulty::Medium.holes() < Difficulty::Hard.holes());
    }

    #[test]
    fn filled_mask_matches_nonzero() {
        let mut rng = StdRng::seed_from_u64(3);
        let p = generate(Difficulty::Hard, &mut rng);
        let mask = filled_mask(&p.givens);
        for r in 0..SIZE {
            for c in 0..SIZE {
                assert_eq!(mask[r][c], p.givens[r][c] != 0);
            }
        }
    }
}
