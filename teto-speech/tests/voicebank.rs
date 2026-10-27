#![cfg(feature = "voicebank-wav")]
use std::{fs, io::Write, path::Path};
use teto_speech::{
    error::SpeechError,
    voicebank::{
        OtoRowId, PitchId, SourceId,
        oto::{import_voicebank, parse_row},
    },
};
fn wav(path: &Path) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..4410 {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
}
fn fixture(oto: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    wav(&dir.path().join("unit.wav"));
    fs::write(dir.path().join("oto.ini"), oto).unwrap();
    dir
}
#[test]
fn oto_import_preserves_signed_timing_duplicates_and_sources() {
    let dir = fixture("unit.wav=- a,1.001,10,-50,5,-2.125\nunit.wav=- a,2,10,40,5,1\n");
    let imported = import_voicebank(dir.path()).unwrap();
    assert_eq!(imported.report.sources, 1);
    assert_eq!(imported.report.rows, 2);
    assert_eq!(imported.report.duplicate_rows, 1);
    let bank = imported.index;
    let (id, row) = bank.find("- a", PitchId(0)).unwrap();
    assert_eq!(id, OtoRowId(0));
    assert_eq!(row.line, 1);
    assert_eq!(row.offset_us, 1001);
    assert_eq!(row.consonant_us, 10000);
    assert_eq!(row.overlap_us, -2125);
    assert_eq!(row.end_us, 51001);
    assert_eq!(bank.oto()[1].end_us, 60000);
    assert_eq!(bank.sources()[0].data_length_bytes, 8820);
    assert_eq!(bank.source_path(SourceId(0)), Some("unit.wav"));
}
#[test]
fn archive_and_directory_have_identical_manifest_revision() {
    let dir = fixture("unit.wav=- a,0,10,-50,5,1\n");
    let archive = tempfile::NamedTempFile::new().unwrap();
    let mut zip = zip::ZipWriter::new(archive.reopen().unwrap());
    for name in ["unit.wav", "oto.ini"] {
        zip.start_file(
            format!("bank/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&fs::read(dir.path().join(name)).unwrap())
            .unwrap();
    }
    zip.finish().unwrap();
    let a = import_voicebank(dir.path()).unwrap().index;
    let b = import_voicebank(archive.path()).unwrap().index;
    assert_eq!(a.revision(), b.revision());
    assert_eq!(a.inventory(), b.inventory());
    assert!(b.archive_sha256().is_some());
}
#[test]
fn oto_import_rejects_traversal_overflow_and_invalid_ranges() {
    for source in ["../unit.wav", "/unit.wav", "C:\\unit.wav", "..\\unit.wav"] {
        let dir = fixture(&format!("{source}=- a,0,10,-50,5,1\n"));
        assert!(import_voicebank(dir.path()).is_err());
    }
    for numbers in [
        "0,10,-101,5,1",
        "60,10,50,5,1",
        "0,60,-50,5,1",
        "0,10,-50,51,1",
        "999999999999999999999,10,-50,5,1",
        "NaN,10,-50,5,1",
    ] {
        let dir = fixture(&format!("unit.wav=- a,{numbers}\n"));
        assert!(import_voicebank(dir.path()).is_err());
    }
    assert!(parse_row("unit.wav=- a,0,10,-50,5,1,extra").is_err());
}
#[test]
fn oto_import_preserves_pitch_groups() {
    let dir = fixture("unit.wav=- a,0,10,-50,5,1\n");
    let group = dir.path().join("high");
    fs::create_dir(&group).unwrap();
    wav(&group.join("unit.wav"));
    fs::write(group.join("oto.ini"), "unit.wav=- a_high,1,10,-50,5,1\n").unwrap();
    fs::write(group.join("prefix.map"), "A4\t\t_high\n").unwrap();
    let bank = import_voicebank(dir.path()).unwrap().index;
    assert_eq!(bank.pitches().len(), 2);
    assert!(bank.find("- a", PitchId(0)).is_some());
    assert!(bank.find("- a", PitchId(1)).is_some());
    assert_eq!(bank.pitches()[0].mappings[0].suffix, "_high");
}
#[cfg(unix)]
#[test]
fn oto_import_rejects_symbolic_links() {
    let dir = fixture("unit.wav=- a,0,10,-50,5,1\n");
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::remove_file(dir.path().join("unit.wav")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("unit.wav")).unwrap();
    assert!(matches!(
        import_voicebank(dir.path()),
        Err(SpeechError::VoicebankInvalid)
    ));
}
#[test]
#[ignore = "requires the local user-supplied Teto archive"]
fn real_bank_import_matches_inventory() {
    let path = std::env::var_os("TETO_VOICEBANK_DIR").unwrap();
    let imported = import_voicebank(Path::new(&path)).unwrap();
    assert_eq!(
        (
            imported.report.sources,
            imported.report.rows,
            imported.report.aliases,
            imported.report.duplicate_rows
        ),
        (558, 2681, 2673, 8)
    );
    assert!(imported.index.find("V", PitchId(0)).is_some());
    assert!(imported.index.find("bV", PitchId(0)).is_none());
}

#[test]
fn oto_bytes_decode_cp932_and_reject_broken_encoding() {
    let (bytes, _, bad) = encoding_rs::SHIFT_JIS.encode("unit.wav=テ,0,10,-50,5,-1\n");
    assert!(!bad);
    let mut count = 0;
    teto_speech::voicebank::oto::visit_rows(&bytes, |_, row| {
        assert_eq!(row.alias, "テ");
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 1);
    assert!(teto_speech::voicebank::oto::visit_rows(&[0x81], |_, _| Ok(())).is_err());
}
#[test]
fn prefix_map_retains_distinct_recorded_pitch_groups() {
    let dir = fixture("unit.wav=- a_low,0,10,-50,5,1\nunit.wav=- a_high,0,10,-50,5,1\n");
    fs::write(dir.path().join("prefix.map"), "A3\t\t_low\nA4\t\t_high\n").unwrap();
    let bank = import_voicebank(dir.path()).unwrap().index;
    assert_eq!(bank.pitches().len(), 2);
    for (index, group) in bank.pitches().iter().enumerate() {
        let (_, row) = bank.find("- a", PitchId(index as u32)).unwrap();
        assert_eq!(
            bank.alias_text(row.alias).unwrap(),
            format!("- a{}", group.mappings[0].suffix)
        );
        assert!(!bank.alias_symbols(row.alias).unwrap().is_empty());
    }
}
