// Seeds tied audit tuples only in the isolated production-handler proof process.
// SDK requests still execute the actual authenticated Go audit handler and storage query.
package main

import (
	"github.com/labstack/echo/v4"
	"net/http"
	"papo/internal/storage"
)

func registerOperationsProof(e *echo.Echo) {
	e.POST("/__proof/operations-audit", func(c echo.Context) error {
		_, err := storage.GetDB().ExecContext(c.Request().Context(), `
   INSERT INTO audit_logs (id, actor_id, actor_username, action, entity_type, metadata, created_at)
   SELECT ('00000000-0000-4000-9000-' || lpad(n::text, 12, '0'))::uuid,
          u.id, u.username, 'sdk.proof.delete', 'sdk_proof',
          jsonb_build_object('channel_id','00000000-0000-4000-8000-000000000013'),
          TIMESTAMPTZ '2026-01-01 00:00:00.123456+00'
   FROM generate_series(1,105) n CROSS JOIN users u WHERE u.username='cursorproof'`)
		if err != nil {
			return c.NoContent(http.StatusInternalServerError)
		}
		return c.NoContent(http.StatusNoContent)
	})
}
