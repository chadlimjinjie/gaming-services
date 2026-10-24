// Package game holds the authoritative Connect Four rules. This is the source
// of truth; the TypeScript copy in the frontend mirrors it only for instant
// client-side feedback.
package game

const (
	Rows = 6
	Cols = 7
)

// Cell is the occupant of a board square. Empty is the JSON zero value "".
type Cell string

const (
	Empty  Cell = ""
	Red    Cell = "red"
	Yellow Cell = "yellow"
)

// Board is indexed [row][col] with row 0 at the top and row Rows-1 at the
// bottom. Discs fall to the lowest empty row in a column.
type Board [Rows][Cols]Cell

func inBounds(r, c int) bool {
	return r >= 0 && r < Rows && c >= 0 && c < Cols
}

// ApplyMove drops a disc of the given color into col. It returns the row the
// disc landed in and true, or false if the column is out of range or full.
func (b *Board) ApplyMove(col int, color Cell) (int, bool) {
	if col < 0 || col >= Cols {
		return 0, false
	}
	for row := Rows - 1; row >= 0; row-- {
		if b[row][col] == Empty {
			b[row][col] = color
			return row, true
		}
	}
	return 0, false
}

// DetectWin checks whether the disc at (row,col) completes a run of four or
// more. It returns the winning cells (as [row,col] pairs) or nil.
func (b *Board) DetectWin(row, col int) [][2]int {
	color := b[row][col]
	if color == Empty {
		return nil
	}
	dirs := [4][2]int{{0, 1}, {1, 0}, {1, 1}, {1, -1}}
	for _, d := range dirs {
		cells := [][2]int{{row, col}}
		for k := 1; ; k++ {
			rr, cc := row+d[0]*k, col+d[1]*k
			if !inBounds(rr, cc) || b[rr][cc] != color {
				break
			}
			cells = append(cells, [2]int{rr, cc})
		}
		for k := 1; ; k++ {
			rr, cc := row-d[0]*k, col-d[1]*k
			if !inBounds(rr, cc) || b[rr][cc] != color {
				break
			}
			cells = append(cells, [2]int{rr, cc})
		}
		if len(cells) >= 4 {
			return cells
		}
	}
	return nil
}

// IsFull reports whether the board has no empty squares left (a draw once no
// win exists). Checking the top row suffices because discs stack upward.
func (b *Board) IsFull() bool {
	for c := 0; c < Cols; c++ {
		if b[0][c] == Empty {
			return false
		}
	}
	return true
}
