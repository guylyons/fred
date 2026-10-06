use super::*;

fn run(o: &mut Options, args: &str) -> Result<String, String> {
    set(o, args, Level::Both)
}

#[test]
fn table_and_lookup() {
    // `Opt` and `OPTIONS` are in the same order.
    for (i, d) in OPTIONS.iter().enumerate() {
        assert_eq!(Opt::ALL[i].def().name, d.name);
        assert_eq!(Opt::find(d.name), Some(Opt::ALL[i]));
    }
    assert_eq!(Opt::find("ts"), Some(Opt::Tabstop));
    assert_eq!(Opt::find("nu"), Some(Opt::Number));
    assert_eq!(Opt::find("tab"), None);
}

#[test]
fn set_forms() {
    let mut o = Options::default();
    assert_eq!(run(&mut o, "ts=4"), Ok(String::new()));
    assert_eq!(o.num(Opt::Tabstop), 4);
    run(&mut o, "ts+=2").unwrap();
    assert_eq!(o.num(Opt::Tabstop), 6);
    run(&mut o, "ts-=1 ts^=2").unwrap();
    assert_eq!(o.num(Opt::Tabstop), 10);
    run(&mut o, "tabstop:3").unwrap();
    assert_eq!(o.num(Opt::Tabstop), 3);
    run(&mut o, "ts=0x10").unwrap();
    assert_eq!(o.num(Opt::Tabstop), 16);
    run(&mut o, "ts&").unwrap();
    assert_eq!(o.num(Opt::Tabstop), 8);
    // Booleans: name, no, inv, !.
    run(&mut o, "wrap").unwrap();
    assert!(o.bool(Opt::Wrap));
    run(&mut o, "nowrap").unwrap();
    assert!(!o.bool(Opt::Wrap));
    run(&mut o, "invwrap").unwrap();
    assert!(o.bool(Opt::Wrap));
    run(&mut o, "wrap!").unwrap();
    assert!(!o.bool(Opt::Wrap));
    run(&mut o, "nonu rnu").unwrap();
    assert!(!o.bool(Opt::Number) && o.bool(Opt::Relativenumber));
    // Showing values.
    assert_eq!(
        run(&mut o, "ts? wrap? nu?"),
        Ok("tabstop=8  nowrap  nonumber".into())
    );
    assert_eq!(run(&mut o, "ts"), Ok("tabstop=8".into()));
}

#[test]
fn list_options() {
    let mut o = Options::default();
    run(&mut o, "cb=").unwrap();
    assert_eq!(o.str(Opt::Clipboard), "");
    run(&mut o, "cb+=unnamed").unwrap();
    run(&mut o, "cb+=unnamedplus").unwrap();
    run(&mut o, "cb+=unnamed").unwrap();
    assert_eq!(o.str(Opt::Clipboard), "unnamed,unnamedplus");
    run(&mut o, "cb-=unnamed").unwrap();
    assert_eq!(o.str(Opt::Clipboard), "unnamedplus");
    run(&mut o, "cb^=unnamed").unwrap();
    assert_eq!(o.str(Opt::Clipboard), "unnamed,unnamedplus");
}

#[test]
fn errors() {
    let mut o = Options::default();
    assert_eq!(
        run(&mut o, "nosuch"),
        Err("E518: Unknown option: nosuch".into())
    );
    assert_eq!(
        run(&mut o, "nots"),
        Err("E518: Unknown option: nots".into())
    );
    assert_eq!(
        run(&mut o, "ts=x"),
        Err("E521: Number required after =: ts=x".into())
    );
    assert_eq!(
        run(&mut o, "ts=0"),
        Err("E487: Argument must be positive".into())
    );
    assert_eq!(
        run(&mut o, "wrap=1"),
        Err("E474: Invalid argument: wrap=1".into())
    );
    assert_eq!(
        run(&mut o, "cb=bogus"),
        Err("E474: Invalid argument: bogus".into())
    );
    // Arguments before the bad one still apply.
    assert!(run(&mut o, "ts=4 nosuch").is_err());
    assert_eq!(o.num(Opt::Tabstop), 4);
}

#[test]
fn local_and_global_values() {
    let mut a = Options::default();
    // A second buffer: shares globals, has its own local values.
    let mut b = a.for_new_buffer();
    set(&mut a, "ts=4", Level::Local).unwrap();
    assert_eq!((a.num(Opt::Tabstop), b.num(Opt::Tabstop)), (4, 8));
    // `:setglobal` changes what new buffers get, not this one.
    set(&mut a, "ts=2", Level::Global).unwrap();
    assert_eq!(a.num(Opt::Tabstop), 4);
    assert_eq!(set(&mut a, "ts?", Level::Global), Ok("tabstop=2".into()));
    assert_eq!(a.for_new_buffer().num(Opt::Tabstop), 2);
    // `:set` on a global option reaches every buffer.
    set(&mut b, "noac", Level::Both).unwrap();
    assert!(!a.bool(Opt::Autocomplete));
    // A new buffer keeps the window's window options.
    set(&mut a, "wrap", Level::Local).unwrap();
    assert!(a.for_new_buffer().bool(Opt::Wrap));
    // `:setlocal ts<` takes the global value.
    set(&mut a, "ts<", Level::Local).unwrap();
    assert_eq!(a.num(Opt::Tabstop), 2);
    // `:set` lists what differs from the default.
    let mut c = Options::default();
    set(&mut c, "ts=3", Level::Both).unwrap();
    assert_eq!(set(&mut c, "", Level::Both), Ok("tabstop=3".into()));
}
