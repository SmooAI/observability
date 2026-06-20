package observability

import (
	"context"
	"sync"
	"time"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/metric"
)

// Metrics — OpenTelemetry Meter wrapper. A thin Smoo-flavored API over the OTel
// metrics surface, mirroring metrics/index.ts. Instruments are cached by
// (meterName, instrumentName[, unit]) so Meter handles don't leak. Every method
// swallows errors — observability must not panic the host.

// MetricsClient is the metrics emission surface.
type MetricsClient interface {
	// Counter adds to a monotonic counter (default value 1).
	Counter(name string, value float64, attrs map[string]string)
	// Histogram records a distribution observation.
	Histogram(name string, value float64, attrs map[string]string)
	// Timing records a duration in ms (histogram with unit "ms").
	Timing(name string, ms float64, attrs map[string]string)
	// StartTimer returns a stop func that records elapsed ms on call.
	StartTimer(name string, attrs map[string]string) func()
	// WithTiming wraps fn, recording elapsed ms with status=success|error.
	WithTiming(ctx context.Context, name string, fn func() error, attrs map[string]string) error
}

type metricsClient struct {
	meterName string
}

var (
	instrumentMu   sync.Mutex
	counterCache   = map[string]metric.Float64Counter{}
	histogramCache = map[string]metric.Float64Histogram{}
)

// GetMetricsClient returns a metrics client bound to a meter name. Pass "" to
// use the default meter "@smooai/observability-go". Cheap to call per service.
func GetMetricsClient(meterName string) MetricsClient {
	if meterName == "" {
		meterName = "@smooai/observability-go"
	}
	return &metricsClient{meterName: meterName}
}

func getCounter(meterName, name string) (metric.Float64Counter, error) {
	key := meterName + "::" + name
	instrumentMu.Lock()
	defer instrumentMu.Unlock()
	if c, ok := counterCache[key]; ok {
		return c, nil
	}
	c, err := otel.Meter(meterName).Float64Counter(name)
	if err != nil {
		return nil, err
	}
	counterCache[key] = c
	return c, nil
}

func getHistogram(meterName, name, unit string) (metric.Float64Histogram, error) {
	key := meterName + "::" + name + "::" + unit
	instrumentMu.Lock()
	defer instrumentMu.Unlock()
	if h, ok := histogramCache[key]; ok {
		return h, nil
	}
	var opts []metric.Float64HistogramOption
	if unit != "" {
		opts = append(opts, metric.WithUnit(unit))
	}
	h, err := otel.Meter(meterName).Float64Histogram(name, opts...)
	if err != nil {
		return nil, err
	}
	histogramCache[key] = h
	return h, nil
}

func toMetricAttrs(attrs map[string]string) metric.MeasurementOption {
	if len(attrs) == 0 {
		return metric.WithAttributes()
	}
	kvs := make([]attribute.KeyValue, 0, len(attrs))
	for k, v := range attrs {
		kvs = append(kvs, attribute.String(k, v))
	}
	return metric.WithAttributes(kvs...)
}

func (m *metricsClient) Counter(name string, value float64, attrs map[string]string) {
	defer recoverSilently()
	if value == 0 {
		value = 1
	}
	c, err := getCounter(m.meterName, name)
	if err != nil {
		return
	}
	c.Add(context.Background(), value, toMetricAttrs(attrs))
}

func (m *metricsClient) Histogram(name string, value float64, attrs map[string]string) {
	defer recoverSilently()
	h, err := getHistogram(m.meterName, name, "")
	if err != nil {
		return
	}
	h.Record(context.Background(), value, toMetricAttrs(attrs))
}

func (m *metricsClient) Timing(name string, ms float64, attrs map[string]string) {
	defer recoverSilently()
	h, err := getHistogram(m.meterName, name, "ms")
	if err != nil {
		return
	}
	h.Record(context.Background(), ms, toMetricAttrs(attrs))
}

func (m *metricsClient) StartTimer(name string, attrs map[string]string) func() {
	start := time.Now()
	return func() {
		defer recoverSilently()
		ms := float64(time.Since(start).Milliseconds())
		h, err := getHistogram(m.meterName, name, "ms")
		if err != nil {
			return
		}
		h.Record(context.Background(), ms, toMetricAttrs(attrs))
	}
}

func (m *metricsClient) WithTiming(ctx context.Context, name string, fn func() error, attrs map[string]string) error {
	start := time.Now()
	err := fn()
	ms := float64(time.Since(start).Milliseconds())

	status := "success"
	if err != nil {
		status = "error"
	}
	merged := make(map[string]string, len(attrs)+1)
	for k, v := range attrs {
		merged[k] = v
	}
	merged["status"] = status

	func() {
		defer recoverSilently()
		if h, gErr := getHistogram(m.meterName, name, "ms"); gErr == nil {
			h.Record(ctx, ms, toMetricAttrs(merged))
		}
	}()
	return err
}

// resetMetricsInstrumentCache is a test seam — drops cached instruments so a
// fresh MeterProvider takes effect.
func resetMetricsInstrumentCache() {
	instrumentMu.Lock()
	defer instrumentMu.Unlock()
	counterCache = map[string]metric.Float64Counter{}
	histogramCache = map[string]metric.Float64Histogram{}
}
