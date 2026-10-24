package game

import "testing"

// drop plays a disc and returns the landing row, failing the test if illegal.
func drop(t *testing.T, b *Board, col int, c Cell) int {
	t.Helper()
	row, ok := b.ApplyMove(col, c)
	if !ok {
		t.Fatalf("unexpected illegal move into col %d", col)
	}
	return row
}

func TestApplyMoveStacks(t *testing.T) {
	var b Board
	if r := drop(t, &b, 3, Red); r != Rows-1 {
		t.Fatalf("first disc landed in row %d, want %d", r, Rows-1)
	}
	if r := drop(t, &b, 3, Yellow); r != Rows-2 {
		t.Fatalf("second disc landed in row %d, want %d", r, Rows-2)
	}
}

func TestApplyMoveRejectsFullColumn(t *testing.T) {
	var b Board
	for i := 0; i < Rows; i++ {
		drop(t, &b, 0, Red)
	}
	if _, ok := b.ApplyMove(0, Red); ok {
		t.Fatal("expected full column to reject move")
	}
}

func TestDetectWinHorizontal(t *testing.T) {
	var b Board
	var last int
	for c := 0; c < 4; c++ {
		last = drop(t, &b, c, Red)
	}
	if win := b.DetectWin(last, 3); len(win) < 4 {
		t.Fatalf("expected horizontal win, got %v", win)
	}
}

func TestDetectWinVertical(t *testing.T) {
	var b Board
	var last int
	for i := 0; i < 4; i++ {
		last = drop(t, &b, 2, Yellow)
	}
	if win := b.DetectWin(last, 2); len(win) < 4 {
		t.Fatalf("expected vertical win, got %v", win)
	}
}

func TestDetectWinDiagonal(t *testing.T) {
	var b Board
	// Build a / diagonal of Red in columns 0..3.
	drop(t, &b, 0, Red)
	drop(t, &b, 1, Yellow)
	drop(t, &b, 1, Red)
	drop(t, &b, 2, Yellow)
	drop(t, &b, 2, Yellow)
	drop(t, &b, 2, Red)
	drop(t, &b, 3, Yellow)
	drop(t, &b, 3, Yellow)
	drop(t, &b, 3, Yellow)
	row := drop(t, &b, 3, Red)
	if win := b.DetectWin(row, 3); len(win) < 4 {
		t.Fatalf("expected diagonal win, got %v", win)
	}
}

func TestNoFalseWin(t *testing.T) {
	var b Board
	row := drop(t, &b, 0, Red)
	if win := b.DetectWin(row, 0); win != nil {
		t.Fatalf("single disc should not win, got %v", win)
	}
}

func TestIsFull(t *testing.T) {
	var b Board
	if b.IsFull() {
		t.Fatal("empty board reported full")
	}
	colors := []Cell{Red, Yellow}
	for c := 0; c < Cols; c++ {
		for r := 0; r < Rows; r++ {
			// Alternate so no four-in-a-row, just fill it.
			drop(t, &b, c, colors[(c+r)%2])
		}
	}
	if !b.IsFull() {
		t.Fatal("completely filled board not reported full")
	}
}
