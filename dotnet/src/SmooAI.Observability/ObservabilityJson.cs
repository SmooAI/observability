using System.Text.Json;
using System.Text.Json.Serialization;

namespace SmooAI.Observability;

/// <summary>
/// Shared <see cref="JsonSerializerOptions"/> for the wire format. camelCase
/// property names + omit-nulls reproduce the TS SDK's <c>JSON.stringify</c>
/// output (TS omits <c>undefined</c> fields and uses camelCase keys), so the
/// ingest endpoint sees byte-compatible payloads from either SDK.
/// </summary>
public static class ObservabilityJson
{
    /// <summary>The canonical serializer options for ingest payloads.</summary>
    public static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        // The dashboard does not need indentation; keep payloads compact.
        WriteIndented = false,
    };

    /// <summary>Serialize an ingest payload to the wire JSON string.</summary>
    public static string Serialize(IngestPayload payload) => JsonSerializer.Serialize(payload, Options);
}
