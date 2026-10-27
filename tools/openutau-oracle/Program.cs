using System.Text.Json;
using OpenUtau.Api;
using OpenUtau.Core.Ustx;
using OpenUtau.Plugin.Builtin;

var document = JsonDocument.Parse(File.ReadAllText(args[0]));
var aliases = document.RootElement.GetProperty("inventory").EnumerateArray().Select(x => x.GetString()).ToArray();
var reference = new Reference(new InventorySinger(aliases));
var records = new List<object>();
foreach (var record in document.RootElement.GetProperty("records").EnumerateArray()) {
    if (!record.TryGetProperty("phones", out var phones)) continue;
    var symbols = phones.EnumerateArray().Select(p => p.GetString().ToLowerInvariant()).ToArray();
    var words = record.GetProperty("words").EnumerateArray().Select(w => w.EnumerateArray().Select(x => x.GetInt32()).ToArray()).ToArray();
    var breaks = record.GetProperty("breaks").EnumerateArray().Select(x => x.GetInt32()).ToHashSet();
    records.Add(new { @case = record.GetProperty("case").GetInt32(), groups = reference.Run(symbols, words, breaks) });
}
File.WriteAllText(args[1], JsonSerializer.Serialize(records));

sealed class InventorySinger : USinger {
    readonly Dictionary<string, UOto> aliases;
    public InventorySinger(string[] inventory) {
        aliases = inventory.ToDictionary(a => a, a => UOto.OfDummy(a));
        found = true;
        loaded = true;
    }
    public override bool TryGetOto(string alias, out UOto oto) => aliases.TryGetValue(alias, out oto);
}

sealed class Reference : EnXSampaPhonemizer {
    public Reference(USinger inventory) { singer = inventory; }
    public List<string[]> Run(string[] phones, int[][] words, HashSet<int> breaks) {
        var notes = words.Select(w => new[] { new Phonemizer.Note {
            lyric = string.Join(" ", phones[w[0]..w[1]]),
            tone = 57, position = w[0] * 96, duration = (w[1] - w[0]) * 96,
            phonemeAttributes = Array.Empty<Phonemizer.PhonemeAttributes>()
        }}).ToArray();
        var groups = new List<string[]>();
        for (int i = 0; i < words.Length; i++) {
            bool start = i == 0 || breaks.Contains(words[i - 1][1] - 1);
            bool end = i == words.Length - 1 || breaks.Contains(words[i][1] - 1);
            var syllables = MakeSyllables(notes[i], start ? null : MakeEnding(notes[i - 1]), end ? null : MakeEnding(notes[i + 1]));
            foreach (var syllable in syllables) {
                groups.Add(ProcessSyllable(syllable).Where(a => a != null).Select(a => ValidateAlias(a, 57)).ToArray());
            }
            if (end) {
                groups.Add(ProcessEnding(MakeEnding(notes[i]).Value).Where(a => a != null).Select(a => ValidateAlias(a, 57)).ToArray());
            }
        }
        return groups;
    }
}
