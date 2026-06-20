using System.Diagnostics.Metrics;
using SmooAI.Observability.Metrics;

namespace SmooAI.Observability.Tests;

public class MetricsTests
{
    [Fact]
    public void Counter_RecordsMeasurement()
    {
        MetricsClient.ResetForTests();
        var meterName = $"test-{Guid.NewGuid()}";
        var total = 0L;
        using var listener = new MeterListener();
        listener.InstrumentPublished = (inst, l) =>
        {
            if (inst.Meter.Name == meterName)
            {
                l.EnableMeasurementEvents(inst);
            }
        };
        listener.SetMeasurementEventCallback<long>((_, value, _, _) => total += value);
        listener.Start();

        var metrics = MetricsClient.Get(meterName);
        metrics.Counter("requests", 3);
        metrics.Counter("requests");

        listener.RecordObservableInstruments();
        Assert.Equal(4, total);
    }

    [Fact]
    public void Histogram_And_Timing_Record()
    {
        MetricsClient.ResetForTests();
        var meterName = $"test-{Guid.NewGuid()}";
        var values = new List<double>();
        using var listener = new MeterListener();
        listener.InstrumentPublished = (inst, l) =>
        {
            if (inst.Meter.Name == meterName)
            {
                l.EnableMeasurementEvents(inst);
            }
        };
        listener.SetMeasurementEventCallback<double>((_, value, _, _) => values.Add(value));
        listener.Start();

        var metrics = MetricsClient.Get(meterName);
        metrics.Histogram("size", 12.5);
        metrics.Timing("latency", 42);

        Assert.Contains(12.5, values);
        Assert.Contains(42d, values);
    }

    [Fact]
    public void StartTimer_RecordsOnDispose()
    {
        MetricsClient.ResetForTests();
        var meterName = $"test-{Guid.NewGuid()}";
        var recorded = false;
        using var listener = new MeterListener();
        listener.InstrumentPublished = (inst, l) =>
        {
            if (inst.Meter.Name == meterName)
            {
                l.EnableMeasurementEvents(inst);
            }
        };
        listener.SetMeasurementEventCallback<double>((_, _, _, _) => recorded = true);
        listener.Start();

        var metrics = MetricsClient.Get(meterName);
        using (metrics.StartTimer("op"))
        {
            // work
        }

        Assert.True(recorded);
    }

    [Fact]
    public async Task WithTiming_RecordsStatusAndRethrows()
    {
        MetricsClient.ResetForTests();
        var meterName = $"test-{Guid.NewGuid()}";
        var tagStatuses = new List<string?>();
        using var listener = new MeterListener();
        listener.InstrumentPublished = (inst, l) =>
        {
            if (inst.Meter.Name == meterName)
            {
                l.EnableMeasurementEvents(inst);
            }
        };
        listener.SetMeasurementEventCallback<double>((_, _, tags, _) =>
        {
            foreach (var t in tags)
            {
                if (t.Key == "status")
                {
                    tagStatuses.Add(t.Value as string);
                }
            }
        });
        listener.Start();

        var metrics = MetricsClient.Get(meterName);
        var ok = await metrics.WithTimingAsync("work", async () => { await Task.Yield(); return 7; });
        Assert.Equal(7, ok);

        await Assert.ThrowsAsync<InvalidOperationException>(() =>
            metrics.WithTimingAsync<int>("work", () => throw new InvalidOperationException("boom")));

        Assert.Contains("success", tagStatuses);
        Assert.Contains("error", tagStatuses);
    }

    [Fact]
    public void Counter_NeverThrows()
    {
        // Even with an absurd name, must not throw.
        MetricsClient.Get().Counter("a.b.c");
    }
}
