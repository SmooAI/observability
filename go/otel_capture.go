package observability

import (
	"context"
	"errors"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/trace"
)

// OTel-native capture handler — the Go port of node/otel-capture.ts. Registered
// as a Client CaptureHandler so every captured exception/message becomes a span
// event on the active span (or a synthetic one), with status ERROR for
// exceptions and OTLP-shaped attributes for tags/user/release. Fires IN
// ADDITION to the HTTP transport when both are wired (SMOODEV-1148 parity).

// RegisterOtelCapture wires the OTel-native capture path onto the given client.
// ctxProvider supplies the context whose active span is used when the capture
// site didn't carry one (e.g. global handlers). tracerName defaults to
// "smooai.observability".
func RegisterOtelCapture(c *Client, tracerName string) {
	if tracerName == "" {
		tracerName = "smooai.observability"
	}
	tracer := otel.Tracer(tracerName)

	c.RegisterCaptureHandler(func(event ObservabilityEvent, raw RawCapture) {
		defer recoverSilently()
		// The capture site's context isn't threaded into the handler, so use a
		// background context — the active span (if any) is read off the global
		// context the OTel SDK maintains for the current goroutine via the
		// span passed through. Since Go has no implicit ambient span, callers
		// who want span correlation should use CaptureExceptionWithSpan below.
		recordOnTracer(context.Background(), tracer, event, raw)
	})
}

// recordOnTracer records the event on the active span in ctx, or mints a
// synthetic span when none is active.
func recordOnTracer(ctx context.Context, tracer trace.Tracer, event ObservabilityEvent, raw RawCapture) {
	span := trace.SpanFromContext(ctx)
	if span.SpanContext().IsValid() {
		recordOnSpan(span, event, raw)
		return
	}
	name := "observability.captureMessage"
	if len(event.Exception) > 0 {
		name = "observability.captureException"
	}
	_, synthetic := tracer.Start(ctx, name)
	defer synthetic.End()
	recordOnSpan(synthetic, event, raw)
}

func recordOnSpan(span trace.Span, event ObservabilityEvent, raw RawCapture) {
	isException := len(event.Exception) > 0

	attrs := []attribute.KeyValue{attribute.String("smoo.event_id", event.EventID)}
	if event.Environment != "" {
		attrs = append(attrs, attribute.String("deployment.environment.name", event.Environment))
	}
	if event.Release != "" {
		attrs = append(attrs, attribute.String("service.version", event.Release))
	}
	if event.Level != "" {
		attrs = append(attrs, attribute.String("smoo.level", string(event.Level)))
	}
	if event.User != nil {
		if event.User.ID != "" {
			attrs = append(attrs, attribute.String("enduser.id", event.User.ID))
		}
		if event.User.OrgID != "" {
			attrs = append(attrs, attribute.String("enduser.org_id", event.User.OrgID))
		}
		if event.User.SessionID != "" {
			attrs = append(attrs, attribute.String("enduser.session_id", event.User.SessionID))
		}
	}
	for k, v := range event.Tags {
		attrs = append(attrs, attribute.String("smoo.tag."+k, v))
	}

	if isException {
		err := raw.Err
		if err == nil {
			msg := "non-error captured"
			if len(event.Exception) > 0 {
				msg = event.Exception[0].Value
			}
			err = errors.New(msg)
		}
		span.RecordError(err)
		span.SetStatus(codes.Error, err.Error())
	} else if event.Message != "" {
		msgAttrs := append([]attribute.KeyValue{attribute.String("smoo.message", event.Message)}, attrs...)
		span.AddEvent("smoo.message", trace.WithAttributes(msgAttrs...))
		if event.Level == LevelError || event.Level == LevelFatal {
			span.SetStatus(codes.Error, event.Message)
		}
	}

	span.SetAttributes(attrs...)
}

// CaptureExceptionOnSpan records err on the active span in ctx (and via the
// client's transport/scope). Use this when you have a request/operation context
// carrying a span so the error attaches to the right trace — Go has no implicit
// ambient span, so this is the span-correlated capture entry point.
func (c *Client) CaptureExceptionOnSpan(ctx context.Context, err error, tags map[string]string) string {
	defer recoverSilently()
	span := trace.SpanFromContext(ctx)
	if span.SpanContext().IsValid() {
		recordOnSpan(span, ObservabilityEvent{
			EventID:     "",
			Level:       LevelError,
			Exception:   []ExceptionInfo{{Value: errString(err)}},
			Environment: optEnv(c),
			Release:     optRelease(c),
			Tags:        tags,
		}, RawCapture{Err: err, Tags: tags})
	}
	return c.CaptureException(ctx, err, tags)
}

func errString(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func optEnv(c *Client) string {
	if o := c.Options(); o != nil {
		return o.Environment
	}
	return ""
}

func optRelease(c *Client) string {
	if o := c.Options(); o != nil {
		return o.Release
	}
	return ""
}
