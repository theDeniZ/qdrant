# Verse numbering across translations

**Rule: the verse number you print must match the words you print.** Quoted
Luther words get Luther numbers; quoted Synodal words get Synodal numbers.

## Two ways to ask

`bible_lookup(ref, bible, numbering=…)`

- **`numbering="edition"`** (default): `ref` is in each translation's *own*
  numbering. `Ps.51.1` in `luther1912` is the superscription "Ein Psalm Davids";
  `Ps.51.3` is "Gott, sei mir gnädig".
- **`numbering="kjv"`**: `ref` is a KJV/English reference, such as a citation from
  an English source. The tool remaps it for translations with a known table
  (`luther1912`, `synodal`, `ukrogienko`; see `kjv_remapped` in
  `bible_list_translations`). Ohienko Psalms are the exception: see the quirks
  table below. Each verse carries `kjv_osis` (what you asked for)
  and `osis` (the edition's number). **Print `osis`.**

```
bible_lookup(ref="Ps.51.1-3", bible=["kjv","luther1912"], numbering="kjv")
→ kjv Ps.51.1 "Have mercy upon me, O God…"
  luther1912 Ps.51.3 (kjv_osis Ps.51.1) "Gott, sei mir gnädig…"
```

## Where the systems differ

- **Psalms:** most psalms with a superscription count it as verse 1 in the Hebrew
  numbering (Luther, and the German tradition generally): +1 in 59 psalms, +2 in
  Ps 51, 52, 54, 60. Synodal and Ohienko also use Septuagint psalm numbers
  (KJV Ps 23 = Synodal Ps 22).
- **Chapter boundaries** move in about 40 OT places: KJV Mal 4:1 = Luther Mal 3:19;
  KJV Gen 31:55 = Luther Gen 32:1; KJV Jonah 1:17 = Hebrew Jonah 2:1.
- **A few NT verse splits** (Acts, 2 Cor in Luther; Rom 16:25–27 is Rom 14:24–26 in Synodal).

## Known index quirks

| Index | Quirk | What to do |
|---|---|---|
| `synodal` | Psalms and Jonah are stored in Synodal numbering, but **Daniel is stored in KJV numbering**. Printed Synodal Daniel is often +1 (`Dan.6.10` → print **6:11**). | Probe the chapter's last verse. If the index stops one short of the Synodal chapter, it is KJV-keyed there and you add 1 when printing. |
| `ukrogienko` | Stored in Ohienko's own numbering throughout: **Septuagint psalm numbers** (`Ps.22.1` = "Господь — то мій Пастир", `Ps.50.3` = "Помилуй мене, Боже"), Hebrew Jonah (`Jonah.2.1` = the fish), `Dan.6.11` = the open window. **`numbering="kjv"` is wrong for Psalms here**: the remap table still uses Hebrew psalm chapters (KJV `Ps.23.1` returns Ps 23:1 "Господня земля"). Daniel and Jonah remap correctly. | For Psalms use `numbering="edition"` with the Septuagint number (KJV Ps N → Ps N−1 for Ps 11–113 and 117–146), check the words, and print `osis`. Elsewhere print `osis` as returned. |
| `schlachter`, `elberfelder1905`, `spanish`, `japkougo`, `korean` | No remap table. `numbering="kjv"` returns the verse *stored* under the KJV key, which may be a neighbouring verse. | Fetch with `numbering="edition"` and a verse or two either side. Pick the verse whose words match, and print its number. |

## Quick check before quoting any OT verse outside KJV

1. `bible_lookup(ref=<KJV ref>, bible=["kjv", <target>], numbering="kjv")`.
2. Compare meaning: the target text must say what the KJV verse says.
3. If it does not, widen the window (`numbering="edition"`, ±2 verses, or the
   whole chapter) and choose the verse that matches.
4. Print the target edition's number (`osis`, corrected for known quirks).
