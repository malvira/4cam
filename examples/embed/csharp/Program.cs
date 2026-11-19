// Plan a marcam project from C#: P/Invoke into the C library, JSON in and out.
//   dotnet run -- examples/knob/knob.marcam.toml
using System.Runtime.InteropServices;
using System.Text.Json;

static class Marcam
{
    const string Lib = "marcam_c";

    [DllImport(Lib)] static extern IntPtr marcam_version();
    [DllImport(Lib)] static extern void marcam_free(IntPtr s);
    [DllImport(Lib)]
    static extern IntPtr marcam_plan_and_post(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string project,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? machine,
        byte[]? mesh, nuint meshLen,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? opts);

    // Every returned string is copied, then released with marcam_free.
    static JsonDocument Take(IntPtr p)
    {
        try { return JsonDocument.Parse(Marshal.PtrToStringUTF8(p) ?? "null"); }
        finally { marcam_free(p); }
    }

    public static JsonDocument Version() => Take(marcam_version());

    public static JsonDocument PlanAndPost(string project, byte[]? mesh = null, string? machine = null) =>
        Take(marcam_plan_and_post(project, machine, mesh, (nuint)(mesh?.Length ?? 0), null));
}

static class Program
{
    static int Main(string[] args)
    {
        if (args.Length < 1) { Console.Error.WriteLine("usage: MarcamEmbed project.json [mesh.stl]"); return 2; }
        Console.WriteLine(Marcam.Version().RootElement.GetRawText());
        var mesh = args.Length > 1 ? File.ReadAllBytes(args[1]) : null;
        using var result = Marcam.PlanAndPost(File.ReadAllText(args[0]), mesh);
        var root = result.RootElement;
        if (root.TryGetProperty("error", out var err)) { Console.Error.WriteLine(err.GetString()); return 1; }
        foreach (var p in root.GetProperty("programs").EnumerateArray())
            Console.WriteLine($"{p.GetProperty("name").GetString()}: {p.GetProperty("lines").GetInt32()} lines, " +
                              $"{p.GetProperty("issues").GetArrayLength()} issue(s)");
        return 0;
    }
}
