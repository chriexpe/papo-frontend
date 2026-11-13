// Runs the pinned backend's production handlers on loopback with disposable PostgreSQL data.
// Secrets are generated in memory. Only DATABASE_URL is required from the environment.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/labstack/echo/v4"
	"papo/internal/config"
	"papo/internal/handlers"
	"papo/internal/storage"
	"papo/internal/websocket"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "backend proof failed")
		os.Exit(1)
	}
}

func run() error {
	database, err := url.Parse(os.Getenv("DATABASE_URL"))
	if err != nil {
		return fmt.Errorf("invalid proof database")
	}
	ip := net.ParseIP(database.Hostname())
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("proof database must be loopback")
	}
	secret := make([]byte, 64)
	if _, err := rand.Read(secret); err != nil {
		return err
	}
	if err := os.Setenv("JWT_SECRET", hex.EncodeToString(secret[:32])); err != nil {
		return err
	}
	if err := os.Setenv("HMAC_SECRET", hex.EncodeToString(secret[32:])); err != nil {
		return err
	}
	cfg := config.LoadConfig()
	if err := storage.InitDB(cfg.DatabaseURL); err != nil {
		return err
	}
	defer storage.CloseDB()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	files, err := filepath.Glob("../migrations/*.sql")
	if err != nil || len(files) == 0 {
		return fmt.Errorf("missing migrations")
	}
	for _, file := range files {
		sql, err := os.ReadFile(file)
		if err != nil {
			return err
		}
		up := strings.Split(string(sql), "-- +goose Down")[0]
		if _, err := storage.GetDB().ExecContext(ctx, up); err != nil {
			return fmt.Errorf("migration failed: %s", filepath.Base(file))
		}
	}
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		return err
	}
	cfg.BaseURL = "http://" + listener.Addr().String()
	hub := websocket.GetHub()
	go hub.Run()
	e := echo.New()
	e.HideBanner = true
	e.HidePort = true
	handlers.RegisterAuthRoutes(e, cfg)
	handlers.RegisterServerRoutes(e, cfg)
	handlers.RegisterChannelRoutes(e, cfg)
	handlers.RegisterMessageRoutes(e, cfg)
	handlers.RegisterEmbedRoutes(e, cfg)
	handlers.RegisterUserRoutes(e, cfg)
	handlers.RegisterRoleRoutes(e, cfg)
	handlers.RegisterDirectMessageRoutes(e, cfg)
	handlers.RegisterWebSocketRoutes(e, cfg)
	e.GET("/health", func(c echo.Context) error { return c.NoContent(204) })
	e.POST("/__proof/seed", func(c echo.Context) error {
		_, err := storage.GetDB().ExecContext(c.Request().Context(), `
            INSERT INTO messages (id, channel_id, author_id, created_at)
            SELECT ('00000000-0000-4000-8000-' || lpad(n::text, 12, '0'))::uuid,
                   c.id, u.id,
                   TIMESTAMPTZ '2026-01-01 00:00:00.123456+00' +
                       CASE WHEN n=1 THEN INTERVAL '-1 microsecond'
                            WHEN n=7 THEN INTERVAL '1 microsecond' ELSE INTERVAL '0' END
            FROM generate_series(1,7) n
            CROSS JOIN channels c CROSS JOIN users u
            WHERE c.name='cursor-proof' AND u.username='cursorproof'`)
		if err != nil {
			return c.NoContent(500)
		}
		return c.NoContent(204)
	})
	// Read by the parent process, never forwarded to logs.
	if err := os.WriteFile(os.Getenv("PAPO_PROOF_ADDRESS_FILE"), []byte(cfg.BaseURL), 0600); err != nil {
		return err
	}
	server := http.Server{Handler: e, ReadHeaderTimeout: 5 * time.Second}
	return server.Serve(listener)
}
