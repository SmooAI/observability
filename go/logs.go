package observability

import (
	"context"
	"log/slog"

	"go.opentelemetry.io/contrib/bridges/otelslog"
	"go.opentelemetry.io/otel/exporters/otlp/otlplog/otlploghttp"
	logglobal "go.opentelemetry.io/otel/log/global"
	sdklog "go.opentelemetry.io/otel/sdk/log"
	"go.opentelemetry.io/otel/sdk/resource"
)

// Logs signal — the third OTLP pipeline alongside traces and metrics, feeding
// the product /v1/logs endpoint. Mirrors the traces/metrics setup exactly: same
// endpoint base, same per-request auth (static header or TokenProvider via
// buildHTTPClient), same resource. App logs reach it through an slog.Handler
// (SlogHandler), which the otelslog bridge maps into OTel log records — reading
// trace_id/span_id from the ACTIVE span context in each Handle(ctx, ...) call,
// so records correlate with the enclosing trace with no manual plumbing.
//
// Product column mapping: severity←record.Level, body←record.Message,
// trace_id/span_id←active span, resource service.name→service_name,
// attrs→parsed_fields.

// buildLoggerProvider constructs an OTLP/HTTP LoggerProvider with a batch
// processor, or returns nil when no endpoint is configured (graceful no-op).
func buildLoggerProvider(ctx context.Context, opts SetupOtelOptions, logEndpoint string, res *resource.Resource) *sdklog.LoggerProvider {
	if logEndpoint == "" {
		return nil
	}
	logOpts := []otlploghttp.Option{otlploghttp.WithEndpointURL(logEndpoint)}
	if len(opts.Headers) > 0 {
		logOpts = append(logOpts, otlploghttp.WithHeaders(opts.Headers))
	}
	if client := buildHTTPClient(opts); client != nil {
		logOpts = append(logOpts, otlploghttp.WithHTTPClient(client))
	}
	exp, err := otlploghttp.New(ctx, logOpts...)
	if err != nil {
		return nil
	}
	return sdklog.NewLoggerProvider(
		sdklog.WithProcessor(sdklog.NewBatchProcessor(exp)),
		sdklog.WithResource(res),
	)
}

// SlogHandler returns an slog.Handler that emits through the global OTel
// LoggerProvider (installed by SetupOtelSDK). Wire it into an application's
// slog.Logger — or into @smooai/logger via logger.SetSlogHandler — so every log
// line becomes an OTLP log record correlated to the active span. When the logs
// signal is disabled the global provider is a no-op and the handler drops
// records silently, so it is always safe to install.
//
// name becomes the OTel InstrumentationScope name (the logger scope). Pass the
// service or component name; empty is accepted.
func SlogHandler(name string) slog.Handler {
	return otelslog.NewHandler(name, otelslog.WithLoggerProvider(logglobal.GetLoggerProvider()))
}
