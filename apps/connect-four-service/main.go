package main

import (
	"encoding/json"
	"log"
	"net/http"
	"os"

	"github.com/joho/godotenv"

	"github.com/midtone/connect-four-service/internal/room"
	"github.com/midtone/connect-four-service/internal/ws"
)

func main() {
	_ = godotenv.Load()

	mgr := room.NewManager()
	wsHandler := ws.NewHandler(mgr)

	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", handleHealth)
	mux.Handle("/connect-four/ws", wsHandler)

	port := getenv("PORT", "4110")
	log.Printf("connect-four-service listening on :%s", port)
	log.Fatal(http.ListenAndServe(":"+port, mux))
}

func handleHealth(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]string{"status": "ok", "service": "connect-four-service"})
}

func getenv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}
