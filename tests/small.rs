//! `data/small.osm.pbf` is `osmium cat data/small.osm -o data/small.osm.pbf` of the hand-written fixture.

use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn small_planet() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("small");
    let _ = std::fs::remove_dir_all(&out);
    let status = Command::new(env!("CARGO_BIN_EXE_osm-termini"))
        .arg("--pbf")
        .arg(root.join("tests/data/small.osm.pbf"))
        .arg("--out")
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());

    let tsv = std::fs::read_to_string(out.join("termini.tsv")).unwrap();
    // Way 101 ("Quay - Quay") has no place names; relation 302's reversed "es" names follow "Propriano";
    // way 108 and relation 301 lose their route labels, way 108's "es" name splits inside its brackets.
    assert_eq!(
        tsv.lines().collect::<Vec<_>>(),
        [
            "lat\tlon\tlang\tname\tids",
            "41.9200000\t8.7400000\t\tAjaccio\tway/105",
            "52.5200000\t13.4000000\t\tBerlin\trelation/301",
            "50.9300000\t1.8100000\t\tCoquelles\tway/102",
            "51.0900000\t1.1400000\t\tFolkestone\tway/102",
            "54.5700000\t11.9300000\t\tGedser\tway/100,way/107",
            "8.6400000\t-83.1700000\t\tGolfito\tway/108",
            "53.5500000\t10.0000000\t\tHamburg Hbf\trelation/300",
            "60.1445126\t21.3141333\t\tKorpo\tway/106",
            "53.8700000\t10.6700000\t\tLübeck Hbf\trelation/300",
            "43.3000000\t5.3600000\t\tMarseille\trelation/302,way/105",
            "8.5300000\t-83.3000000\t\tPenínsula de Osa\tway/108",
            "50.0800000\t14.4300000\t\tPraha\trelation/301",
            "42.6700000\t8.9000000\t\tPropriano\trelation/302",
            "8.5300000\t-83.3000000\t\tPuerto Jiménez\tway/108",
            "54.1800000\t12.0900000\t\tRostock\tway/100,way/107",
            "60.1445126\t21.3141333\t\tVerkan\tway/106",
            "52.5200000\t13.4000000\tde\tBerlin\trelation/301",
            "54.5709000\t11.9300000\tde\tGedser\tway/107",
            "50.0800000\t14.4300000\tde\tPrag\trelation/301",
            "54.1809000\t12.0900000\tde\tRostock\tway/107",
            "8.6400000\t-83.1700000\tes\tGolfito\tway/108",
            "43.3000000\t5.3600000\tes\tMarsella\trelation/302",
            "42.6700000\t8.9000000\tes\tPropriano\trelation/302",
            "8.5300000\t-83.3000000\tes\tPuerto Jiménez\tway/108",
        ]
    );
}
