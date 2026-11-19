//! The session's tests: loading, running, rewinding, input, probes.

use super::*;
use simsandbox_file::FilterEntry;
use simsandbox_views::{History, SignalRef};

mod basics {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn lorenz() -> Session {
        Session::load("lorenz", EXAMPLES[0].text, &registry()).unwrap()
    }

    #[test]
    fn examples_load_and_plot_what_their_controls_ask_for() {
        let s = lorenz();
        assert_eq!(
            s.signals().names(),
            ["lorenz.xyz[0]", "lorenz.xyz[1]", "lorenz.xyz[2]"]
        );
        assert!(s.warnings.is_empty());
        for e in EXAMPLES {
            Session::load_in(e.name, e.text, &registry(), e.folder().as_deref()).unwrap();
        }
    }

    #[test]
    fn rewind_then_rerun_reproduces_the_run() {
        let mut s = lorenz();
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        for _ in 0..300 {
            s.step().unwrap();
        }
        let reference: Vec<_> = s.signals().points(0).to_vec();

        s.rewind_to(1.234);
        // Latest snapshot at or before 1.234 (snapshots every 0.1 s).
        assert!((s.time() - 1.2).abs() < 1e-9, "{}", s.time());
        assert!(s.signals().points(0).last().unwrap().x <= s.time());
        while s.signals().points(0).len() < reference.len() {
            s.step().unwrap();
        }
        assert_eq!(s.signals().points(0), reference.as_slice());

        s.rewind_to(0.0);
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals().points(0).len(), 1);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    fn param(s: &Session, key: &str) -> f64 {
        s.params().iter().find(|r| r.key == key).unwrap().value
    }

    #[test]
    fn initial_conditions_apply_on_restart_or_at_once_before_running() {
        let mut s = lorenz();
        let id = s.ids["lorenz"];
        let row = s
            .params()
            .into_iter()
            .find(|r| r.key == "lorenz.x0")
            .unwrap();
        assert!(row.initial_condition);

        // Before the run starts, the plot's first point moves at once.
        s.set_param(id, "x0", 5.0).unwrap();
        assert_eq!(s.signals().points(0)[0].y, 5.0);

        for _ in 0..50 {
            s.step().unwrap();
        }
        s.set_param(id, "x0", -3.0).unwrap();
        assert!(s.time() > 0.0, "a running model is not restarted");
        s.restart().unwrap();
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals().points(0).len(), 1);
        assert_eq!(s.signals().points(0)[0].y, -3.0);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    #[test]
    fn saving_records_params_step_controls_and_speed() {
        let mut s = lorenz();
        let id = s.ids["lorenz"];
        s.set_param(id, "rho", 150.0).unwrap();
        s.dt = 0.005;
        s.file
            .controls
            .params
            .entry("lorenz.beta".into())
            .or_default()
            .slider = true;
        let text = s.save_json(2.5, None).unwrap();

        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(param(&again, "lorenz.rho"), 150.0);
        assert_eq!(again.dt, 0.005);
        assert_eq!(again.file.controls.speed, Some(2.5));
        assert!(again.file.controls.params["lorenz.beta"].slider);
        assert!(again.file.controls.params["lorenz.rho"].slider);
    }

    #[test]
    fn the_run_pauses_once_at_t_end() {
        let text = EXAMPLES[0]
            .text
            .replace(r#""dt": 0.01"#, r#""dt": 0.01, "t_end": 0.5"#);
        let mut s = Session::load("l", &text, &registry()).unwrap();
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        let mut steps = 0;
        while !s.take_end_reached() {
            s.step().unwrap();
            steps += 1;
        }
        assert_eq!(steps, 50);
        s.step().unwrap();
        assert!(!s.take_end_reached(), "Run continues past t_end");
        // Rewinding before t_end re-arms it.
        s.rewind_to(0.2);
        assert!(!s.take_end_reached());
        while !s.take_end_reached() {
            s.step().unwrap();
        }
        assert!((s.time() - 0.5).abs() < 1e-9);
    }

    fn example(name_part: &str) -> Session {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains(name_part))
            .unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    fn exprs(pairs: &[(&str, &str)]) -> Vec<FilterEntry> {
        pairs
            .iter()
            .map(|(n, e)| FilterEntry::expression(n, e, Vec::new()))
            .collect()
    }

    fn values(s: &Session, signal: &str) -> Vec<f64> {
        let r = s.signals().resolve(signal).unwrap();
        s.signals().values(r)
    }

    /// The newest frame of field `name`.
    fn newest(s: &Session, name: &str) -> Vec<f64> {
        let h = s.signals();
        let f = h.fields().iter().find(|f| f.name == name).unwrap();
        f.frame(h.len() - 1).to_vec()
    }

    #[test]
    fn formula_models_run_in_time_and_their_studies_give_the_curves() {
        // No default sweep (ADR 0071): the resonance curve is a study of
        // the spring, whose damping is an input the slider sets.
        let mut s = example("Resonance curve");
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(s.signals().index(), "t");
        let amp = newest(&s, "curve.spring_amplitude");
        assert_eq!(amp.len(), 600);
        assert_eq!(newest(&s, "curve.w")[599], 3.0);
        // The same numbers as the headless run of the file.
        let rows =
            crate::run::run(&s.save_json(1.0, None).unwrap(), &registry(), None, None).unwrap();
        let headless = rows
            .outputs
            .iter()
            .find(|o| o.name == "curve.spring_amplitude")
            .unwrap();
        assert_eq!(
            headless
                .values
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            amp.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        // Less damping, a taller peak.
        let peak = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
        s.set_param_key("curve.c", 0.05).unwrap();
        s.step().unwrap();
        assert!(peak(&newest(&s, "curve.spring_amplitude")) > 3.0 * peak(&amp));
    }

    #[test]
    fn a_study_given_up_leaves_its_step_undone() {
        // ADR 0071 step 2: the app gives up a study when what it reads
        // changes; the step is undone, and done again later.
        use crate::run::{given_up, watch_studies, Progress};
        let mut s = example("Resonance curve");
        s.step().unwrap();
        let progress = std::sync::Arc::new(Progress::default());
        watch_studies(Some(progress.clone()));
        // Less damping: the next step runs the study again.
        s.set_param_key("curve.c", 0.05).unwrap();
        let (t, len) = (s.time(), s.signals().len());
        progress.give_up();
        let e = s.step().unwrap_err();
        assert!(given_up(&e), "{e}");
        assert_eq!((s.time(), s.signals().len()), (t, len));
        assert_eq!(progress.runs(), None);
        // Done again: every run counted, the same numbers as a model that
        // had the value from the start.
        progress.reset();
        s.step().unwrap();
        assert_eq!(progress.runs(), Some((600, 600)));
        watch_studies(None);
        let mut direct = example("Resonance curve");
        direct.set_param_key("curve.c", 0.05).unwrap();
        assert_eq!(
            newest(&s, "curve.spring_amplitude"),
            newest(&direct, "curve.spring_amplitude")
        );
    }

    #[test]
    fn a_model_built_without_its_studies_runs_them_later() {
        // How the app opens a model (ADR 0071): built and checked on the
        // window's thread, its studies run on the worker.
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Resonance curve"))
            .unwrap();
        let mut s = crate::study::without_running_studies(|| {
            Session::load(e.name, e.text, &registry()).unwrap()
        });
        assert!(s.unevaluated());
        assert!(newest(&s, "curve.spring_amplitude")
            .iter()
            .all(|v| v.is_nan()));
        s.evaluate().unwrap();
        assert!(!s.unevaluated());
        let direct = example("Resonance curve");
        assert_eq!(
            newest(&s, "curve.spring_amplitude"),
            newest(&direct, "curve.spring_amplitude")
        );
        assert!(!direct.unevaluated());
        // A model without studies has none to run.
        let plain = crate::study::without_running_studies(|| example("Lorenz"));
        assert!(!plain.unevaluated());
    }

    #[test]
    fn the_history_knows_each_outputs_unit() {
        let s = example("Tuning");
        let h = s.signals();
        assert_eq!(h.unit("best.best_peak"), Some("m/s"));
        assert_eq!(h.unit("best.best_reach"), Some("s"));
        assert_eq!(h.unit("tuning.reach"), Some("s"));
        drop(h);
        // A plain number has none.
        let l = example("Logistic");
        assert_eq!(l.signals().unit("population.x"), None);
    }

    #[test]
    fn models_of_maps_that_never_read_time_iterate() {
        // ADR 0083: the search iterates; maps that read t, open-ended maps
        // and models with continuous states do not.
        for (part, iterates) in [
            ("Searching", true),
            ("Logistic", false),
            ("Cruise control", false),
            ("Resonance curve", false),
        ] {
            assert_eq!(example(part).iterates(), iterates, "{part}");
        }
        let lines = |eq: &str| {
            format!(
                r#"{{"format": 1, "run": {{"dt": 1, "t_end": 50}}, "blocks": [
                {{"name": "m", "type": "eq.equations", "equations": ["state x = 0", "{eq}"]}}]}}"#
            )
        };
        let load = |text: &str| Session::load("m", text, &registry()).unwrap();
        assert!(load(&lines("x[n+1] = (x + 2/max(x, 1))/2")).iterates());
        assert!(!load(&lines("x[n+1] = x + t")).iterates());
        // It iterates until a round changes nothing: Newton's square root
        // of 2 holds still well before its last round.
        let mut s = load(&lines("x[n+1] = (x + 2/max(x, 1))/2"));
        s.evaluate_later();
        let mut rounds = 0;
        while s.rounds_pending() {
            s.step().unwrap();
            rounds += 1;
            assert!(rounds <= 50, "it ends by its last round");
        }
        assert!(s.held_still() && rounds < 20, "{rounds}");
        assert_eq!(s.round(), rounds);
        // The search holds still before its 40 rounds; a change starts it
        // again from round 0.
        let mut s = example("Searching");
        s.evaluate_later();
        while s.rounds_pending() && !s.take_end_reached() {
            s.step().unwrap();
        }
        assert!(s.held_still() && s.round() < 40, "{}", s.round());
        s.set_param_key("search.limit", 30.0).unwrap();
        assert!(s.rounds_pending() && !s.held_still());
        assert_eq!(s.round(), 0);
    }

    #[test]
    fn models_with_nothing_in_time_are_told_apart() {
        // ADR 0072: no states, tables, data, clocks or schedule.
        for (part, timeless) in [
            ("Resonance curve", true),
            ("Resonance map", true),
            ("Tuning", true),
            ("Units", false),
            ("Lorenz", false),
            ("Logistic", false),
            ("grazing", false),
            ("Steer the car", false),
        ] {
            assert_eq!(example(part).timeless(), timeless, "{part}");
        }
    }

    #[test]
    fn a_model_with_nothing_in_time_can_leave_a_change_for_later() {
        let mut s = example("Resonance curve");
        let before = newest(&s, "curve.spring_amplitude");
        // At once, by default (headless).
        s.set_param_key("curve.c", 0.1).unwrap();
        let at_once = newest(&s, "curve.spring_amplitude");
        assert_ne!(at_once, before);
        assert!(!s.unevaluated());
        // Left for later (the app's worker): the values stay until then.
        s.evaluate_later();
        s.set_param_key("curve.c", 0.05).unwrap();
        assert!(s.unevaluated());
        assert_eq!(newest(&s, "curve.spring_amplitude"), at_once);
        s.evaluate().unwrap();
        assert!(!s.unevaluated());
        let mut direct = example("Resonance curve");
        direct.set_param_key("curve.c", 0.05).unwrap();
        assert_eq!(
            newest(&s, "curve.spring_amplitude"),
            newest(&direct, "curve.spring_amplitude")
        );
        assert_eq!((s.time(), s.signals().len()), (0.0, 1));
    }

    #[test]
    fn a_top_level_sample_is_refused_with_what_to_write() {
        let e = Session::load(
            "old",
            r#"{"format": 1, "sample": {"design": "grid", "params": [{"param": "f.a"}]},
                "blocks": [{"name": "f", "type": "eq.equations", "equations": ["y = a"]}]}"#,
            &registry(),
        )
        .err()
        .unwrap();
        assert!(e.contains("ADR 0071") && e.contains("study"), "{e}");
    }

    #[test]
    fn maps_step_once_per_communication_step() {
        let mut s = example("Logistic");
        s.step().unwrap();
        let x = values(&s, "population.x");
        assert_eq!(x[0], 0.2);
        assert!((x[1] - 3.7 * 0.2 * 0.8).abs() < 1e-15);
        assert_eq!(s.time(), 1.0);
    }

    #[test]
    fn derived_signals_are_recorded_and_can_be_redefined() {
        let mut s = example("Coupled");
        // The example defines the total energy, which stays constant: the
        // two blocks are fused, so there is no coupling error (ADR 0023).
        assert!(s.filters().iter().any(|f| f.name == "energy"));
        for _ in 0..500 {
            s.step().unwrap();
        }
        let e = values(&s, "energy");
        assert_eq!(e.len(), s.signals().len());
        let drift = e.iter().map(|v| (v - e[0]).abs()).fold(0.0, f64::max);
        assert!(drift < 1e-9 * e[0], "energy drifted by {drift} of {}", e[0]);

        // Redefining recomputes the whole history (of what a view reads,
        // ADR 0063).
        s.set_view_reads(vec!["ax2".into()]);
        s.set_filters(exprs(&[("ax2", "a.x^2 + 0*t")])).unwrap();
        let ax = values(&s, "a.x");
        let ax2 = values(&s, "ax2");
        assert_eq!(ax2.len(), ax.len());
        assert!(ax.iter().zip(&ax2).all(|(x, y)| (x * x - y).abs() < 1e-15));
        assert!(s.signals().resolve("energy").is_none());

        // Problems leave everything as it was.
        let problems = s
            .set_filters(exprs(&[("b", "nope + 1"), ("a.x", "1")]))
            .unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("b:")), "{problems:?}");
        assert!(
            problems.iter().any(|p| p.starts_with("a.x:")),
            "{problems:?}"
        );
        assert!(s.signals().resolve("ax2").is_some());
        // Saved as a filter that names its inputs (ADR 0032).
        let saved = s.save_json(1.0, None).unwrap();
        let file: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            file["controls"]["filters"],
            serde_json::json!([{"name": "ax2", "type": "expression", "inputs": ["a.x"], "expr": "a.x^2 + 0*t"}])
        );
    }

    #[test]
    fn expression_filters_may_read_each_other() {
        let mut s = example("Coupled");
        s.set_view_reads(vec!["double".into()]);
        s.set_filters(exprs(&[("double", "2*sq"), ("sq", "a.x^2")]))
            .unwrap();
        for _ in 0..10 {
            s.step().unwrap();
        }
        let ax = values(&s, "a.x");
        let double = values(&s, "double");
        assert!(ax
            .iter()
            .zip(&double)
            .all(|(x, d)| (2.0 * x * x - d).abs() < 1e-14));
        // A cycle is refused.
        assert!(s.set_filters(exprs(&[("p", "q"), ("q", "p")])).is_err());
    }

    #[test]
    fn cut_filters_cut_a_studys_2d_results_into_lines() {
        // A study over two parameters gives 2D fields (ADR 0071) that cuts
        // read directly.
        let mut s = example("Resonance map");
        let cut = default_cut(&s.signals(), "res", "map.spring_amplitude");
        let mut filters = s.filters().to_vec();
        filters.push(cut.clone());
        s.set_filters(filters).unwrap();
        let h = s.signals();
        let a = s.available(&h);
        // Four cuts along w at fixed c, and their coordinate w.
        let lines: Vec<&str> = a
            .lines
            .iter()
            .map(String::as_str)
            .filter(|l| l.starts_with("res"))
            .collect();
        assert_eq!(lines, ["res[0]", "res[1]", "res[2]", "res[3]", "res.x"]);
        assert_eq!(a.axis_choices("res[0]"), ["t", "res.x"]);
        drop(h);
        // What a view of the cut reads is recorded.
        s.set_view_reads(
            ["map.spring_amplitude", "map.c", "map.w", "res[1]", "res.x"]
                .map(String::from)
                .to_vec(),
        );
        s.step().unwrap();
        let (amp, c, w) = (
            newest(&s, "map.spring_amplitude"),
            newest(&s, "map.c"),
            newest(&s, "res.x"),
        );
        let at = cut.settings["at"].as_array().unwrap()[1].as_f64().unwrap();
        // Default cuts lie on rows, so they read them exactly.
        let row = (0..120).find(|&j| c[j * 200] == at).unwrap();
        assert_eq!(
            newest(&s, "res[1]"),
            amp[row * 200..row * 200 + 200].to_vec()
        );
        assert_eq!(w, newest(&s, "map.w")[..200].to_vec());
        let h = s.signals();
        let label = h.label(h.resolve("res[1]").unwrap());
        assert!(label.starts_with("map.c = "), "{label}");
        assert_eq!(h.label(h.resolve("res.x").unwrap()), "map.w");
        drop(h);
        // Saved with its input.
        let saved = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &saved, &registry()).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.filters(), s.filters());
    }

    #[test]
    fn field_outputs_are_recorded_whole_and_offered_to_views() {
        let mut s = example("rod");
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        let h = s.signals();
        let a = s.available(&h);
        assert!(a.scalars.is_empty());
        assert_eq!(a.grids, ["rod.temperature", "rod.position"]);
        assert_eq!(a.block_types, ["eq.heat_rod"]);
        drop(h);
        for _ in 0..20 {
            s.step().unwrap();
        }
        assert_eq!(s.signals().len(), 21);
        assert_eq!(s.signals().fields()[0].frame(20).len(), 50);
        // The hot spot cools.
        let peak = |k: usize| {
            s.signals().fields()[0]
                .frame(k)
                .iter()
                .cloned()
                .fold(0.0, f64::max)
        };
        assert!(peak(20) < peak(0));

        // Rewind and derived signals keep the frames in step with the times.
        s.rewind_to(0.5);
        let frames = |s: &Session| -> Vec<Vec<f64>> {
            (0..s.signals().len())
                .map(|k| s.signals().fields()[0].frame(k).to_vec())
                .collect()
        };
        let before = frames(&s);
        assert_eq!(before.len(), 11);
        s.set_filters(exprs(&[("one", "1 + 0*t")])).unwrap();
        assert_eq!(frames(&s), before);

        let s = example("Orbiting");
        assert_eq!(s.available(&s.signals()).points, ["disc.positions"]);
        assert_eq!(s.signals().fields()[0].frame(0).len(), 400);
    }

    #[test]
    fn bad_files_report_errors() {
        let err = Session::load("x", "{", &registry()).err().unwrap();
        assert!(err.contains("line"), "{err}");
    }
}

mod start_tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn open(part: &str) -> Session {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    fn reopen(s: &mut Session) -> Session {
        let text = s.save_json(1.0, None).unwrap();
        Session::load("again", &text, &registry()).unwrap()
    }

    #[test]
    fn equation_blocks_start_from_now_through_their_initial_parameters() {
        let mut s = open("Cruise");
        s.run_until(60.0).unwrap();
        let (car, cruise) = (s.ids["car"], s.ids["cruise"]);
        let v = s.sim.state(car).to_vec();
        let i = s.sim.state(cruise).to_vec();
        let notes = s.start_from_now().unwrap();
        assert!(notes[0].contains("t = 60"), "{notes:?}");
        assert!(
            notes.iter().any(|n| n.starts_with("car: v0 = ")),
            "{notes:?}"
        );
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.sim.state(car), v);
        assert_eq!(s.sim.state(cruise), i);
        assert!(!s.has_start_states(), "parameters say it");
        // Saved and opened again, it starts there.
        let again = reopen(&mut s);
        assert_eq!(again.sim.state(car), v);
        assert!(!again.file.to_json().contains("\"start\""));
    }

    #[test]
    fn fields_and_populations_save_a_start_state_that_reproduces() {
        let mut s = open("Sheep");
        s.run_until(5.0).unwrap();
        let (grass, sheep) = (s.ids["grass"], s.ids["sheep"]);
        let field = s.sim.state(grass).to_vec();
        let animals = s.sim.current(sheep).tables;
        let notes = s.start_from_now().unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("grass: start state saved (1600 values)")),
            "{notes:?}"
        );
        assert!(s.has_start_states());
        assert_eq!(s.sim.state(grass), field);
        assert_eq!(s.sim.current(sheep).tables, animals);
        // The same run from the saved file as from here.
        s.run_until(2.0).unwrap();
        let mut again = reopen(&mut s);
        assert_eq!(again.sim.state(grass), field);
        again.run_until(2.0).unwrap();
        s.restart().unwrap();
        s.run_until(2.0).unwrap();
        assert_eq!(again.sim.state(grass), s.sim.state(grass));
        assert_eq!(again.sim.current(sheep), s.sim.current(sheep));
        // Forgetting goes back to the parameters' start.
        again.forget_start_states().unwrap();
        assert!(again
            .sim
            .state(grass)
            .iter()
            .all(|&g| g == again.sim.state(grass)[0]));
        assert!(!again.has_start_states());
    }

    #[test]
    fn bad_start_states_are_load_errors_at_their_path() {
        let e = EXAMPLES.iter().find(|e| e.name.contains("Lorenz")).unwrap();
        let text = e.text.replacen(
            "\"type\": \"eq.lorenz\",",
            "\"type\": \"eq.lorenz\", \"start\": {\"state\": [1, 2]},",
            1,
        );
        let check = crate::describe::check(&text, &registry());
        assert_eq!(check["errors"][0]["path"], "blocks[0].start", "{check}");
        let swept = open("Resonance curve");
        let mut swept = swept;
        assert!(swept.start_from_now().is_err());
    }
}

mod subsystem_tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use simsandbox_file::ParamValue;

    /// The model the cars use, as the file defines it (ADR 0068).
    fn follower(f: &ModelFile) -> ModelFile {
        simsandbox_file::compose::read_definition("follower", &f.models["follower"]).unwrap()
    }

    fn platoon() -> Session {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    #[test]
    fn declared_parameters_are_listed_first_and_formulas_are_shown_not_edited() {
        let s = platoon();
        let rows = s.params();
        let keys: Vec<&str> = rows.iter().take(5).map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["ki", "kp", "car1.ki", "car1.kp", "car1.mass"]);
        let car1_kp = rows.iter().find(|r| r.key == "car1.kp").unwrap();
        assert_eq!(car1_kp.formula.as_deref(), Some("kp"));
        assert_eq!(car1_kp.owner, "car1");
        let mass = rows.iter().find(|r| r.key == "car2.mass").unwrap();
        assert!(mass.formula.is_none() && mass.block.is_none());
        let m = rows.iter().find(|r| r.key == "car2/car.m").unwrap();
        assert_eq!((m.value, m.formula.as_deref()), (1500.0, Some("mass")));
        let c = rows.iter().find(|r| r.key == "car2/car.c").unwrap();
        assert!(c.formula.is_none() && c.block.is_some());
    }

    #[test]
    fn changing_a_declared_parameter_reaches_every_block_using_it() {
        let mut s = platoon();
        s.set_param_key("kp", 900.0).unwrap();
        for car in ["car1", "car2", "car3"] {
            let id = s.ids[&format!("{car}/control")];
            assert_eq!(s.sim.model().param(id, "kp").unwrap(), 900.0);
        }
        s.set_param_key("car3.mass", 3000.0).unwrap();
        assert_eq!(s.sim.model().param(s.ids["car3/car"], "m").unwrap(), 3000.0);
        assert!(s
            .set_param_key("car1.kp", 1.0)
            .unwrap_err()
            .contains("computed"));
        assert!(s
            .set_param_key("car1/car.m", 1.0)
            .unwrap_err()
            .contains("computed"));
        s.set_param_key("car1/car.c", 80.0).unwrap();

        // Saved, the formulas stay formulas and only changes are written.
        let text = s.save_json(4.0, None).unwrap();
        let (f, _) = ModelFile::from_json(&text).unwrap();
        assert_eq!(f.params["kp"].value, 900.0);
        let car3 = &f.blocks[3].params;
        assert_eq!(car3["mass"], ParamValue::Number(3000.0));
        assert_eq!(car3["kp"], ParamValue::Formula("kp".into()));
        let car1 = &f.blocks[1].params;
        assert_eq!(car1["car.c"], ParamValue::Number(80.0));
        assert_eq!(car1.len(), 4, "{car1:?}");
        // The definition is untouched: its car still takes c from its
        // equations.
        assert!(!follower(&f).blocks[0].params.contains_key("c"));
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(
            again.sim.model().param(again.ids["car1/car"], "c").unwrap(),
            80.0
        );
        assert_eq!(
            again
                .sim
                .model()
                .param(again.ids["car2/control"], "kp")
                .unwrap(),
            900.0
        );
    }

    #[test]
    fn instances_start_from_a_reached_state_through_their_own_entry() {
        let mut s = platoon();
        s.run_until(20.0).unwrap();
        let v = s.sim.state(s.ids["car3/car"]).to_vec();
        s.start_from_now().unwrap();
        let text = s.save_json(4.0, None).unwrap();
        let (f, _) = ModelFile::from_json(&text).unwrap();
        // The cars' initial speeds are parameters, set for each instance.
        assert_eq!(f.blocks[3].params["car.v0"], ParamValue::Number(v[0]));
        assert!(!follower(&f).blocks[0].params.contains_key("v0"));
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(again.sim.state(again.ids["car3/car"]), v);
    }
}

mod input_tests {
    use super::*;
    use crate::examples::registry;

    /// A cart pushed by an input: dv/dt = F/m - c*v.
    const CART: &str = r#"{"format": 1, "run": {"dt": 0.01},
      "blocks": [
        {"name": "push", "type": "input.value", "settings": {"unit": "N", "min": "-50", "max": "50"}},
        {"name": "cart", "type": "eq.equations", "method": {"name": "rk4"},
         "equations": ["input F [N]", "param m [kg] = 2", "param c [1/s] = 0.5",
                       "state v [m/s]", "dv/dt = F/m - c*v"]}],
      "connections": [{"from": "push.y", "to": "cart.F"}]}"#;

    fn cart(text: &str) -> Session {
        Session::load("cart", text, &registry()).unwrap()
    }

    /// Where two recordings first differ, for a readable failure.
    fn same(a: &[Vec<(u64, u64)>], b: &[Vec<(u64, u64)>], names: &[String]) {
        assert_eq!(a.len(), b.len(), "signal count");
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let k = x.iter().zip(y).position(|(p, q)| p != q);
            assert!(
                k.is_none() && x.len() == y.len(),
                "signal {} differs at sample {:?} (lengths {} and {}): {:?} vs {:?}",
                names[i],
                k,
                x.len(),
                y.len(),
                k.map(|k| (f64::from_bits(x[k].0), f64::from_bits(x[k].1))),
                k.map(|k| (f64::from_bits(y[k].0), f64::from_bits(y[k].1)))
            );
        }
    }

    /// Every recorded signal, as bits.
    fn signals(s: &Session) -> Vec<Vec<(u64, u64)>> {
        let h = s.signals();
        (0..h.names().len())
            .map(|i| {
                h.points(i)
                    .iter()
                    .map(|p| (p.x.to_bits(), p.y.to_bits()))
                    .collect()
            })
            .collect()
    }

    /// Steer by hand: push at 0.5 s, change the mass at 1.2 s, let go at
    /// 2 s; run to 3 s.
    fn steer(s: &mut Session) {
        s.run_until(0.5).unwrap();
        s.set_param_key("push.y", 40.0).unwrap();
        s.run_until(1.2).unwrap();
        s.set_param_key("cart.m", 4.0).unwrap();
        s.run_until(2.0).unwrap();
        s.set_param_key("push.y", 0.0).unwrap();
        s.run_until(3.0).unwrap();
    }

    #[test]
    fn live_input_is_recorded_and_replays_bit_for_bit() {
        let mut s = cart(CART);
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        steer(&mut s);
        let reference = signals(&s);
        assert_eq!(s.recorded().len(), 3);
        // The push moved the cart.
        let v = s.signals().points(1).last().unwrap().y;
        assert!(v > 1.0, "{v}");
        // Rewind and run on: the recorded input is replayed.
        s.rewind_to(0.3);
        assert!(s.replaying());
        s.run_until(3.0).unwrap();
        assert!(!s.replaying());
        same(&signals(&s), &reference, s.signals().names());
        // Saved as a run file, it replays the same in a fresh session.
        let run = s.run_file("cart.ssbx.json", "steer", "", "");
        let (model, _) = ModelFile::from_json(CART).unwrap();
        let loaded = simsandbox_file::load(CART, &registry()).unwrap();
        assert!(run.fits(&loaded).unwrap());
        // A different mass is a different model.
        let heavier = CART.replace("param m [kg] = 2", "param m [kg] = 3");
        let other = simsandbox_file::load(&heavier, &registry()).unwrap();
        assert!(!run.fits(&other).unwrap());
        let mut again = cart(&run.apply_to(&model).to_json());
        let track = run
            .track
            .iter()
            .map(|(t, key, value)| InputEvent {
                t: *t,
                key: key.clone(),
                value: *value,
            })
            .collect();
        again.replay(track).unwrap();
        again.run_until(3.0).unwrap();
        same(&signals(&again), &reference, s.signals().names());
        // Headless, the same track gives the same values.
        let track2 = run
            .track
            .iter()
            .map(|(t, key, value)| InputEvent {
                t: *t,
                key: key.clone(),
                value: *value,
            })
            .collect();
        let text = run.apply_to(&model).to_json();
        let r = crate::run::run_with(&text, &registry(), Some(3.0), None, track2).unwrap();
        let v = r.outputs.iter().find(|o| o.name == "cart.v").unwrap();
        let k = s
            .signals()
            .names()
            .iter()
            .position(|n| n == "cart.v")
            .unwrap();
        let live: Vec<u64> = s
            .signals()
            .points(k)
            .iter()
            .map(|p| p.y.to_bits())
            .collect();
        let headless: Vec<u64> = v.values.iter().map(|x| x.to_bits()).collect();
        assert_eq!(headless, live);
        // The run started from m = 2, not the 4 it ended with.
        let start = &s.start_file().blocks[1].params;
        assert_eq!(
            start.get("m"),
            Some(&simsandbox_file::ParamValue::Number(2.0))
        );
    }

    #[test]
    fn input_acts_in_the_next_step() {
        let mut s = cart(CART);
        s.run_until(0.5).unwrap();
        let k = s
            .signals()
            .names()
            .iter()
            .position(|n| n == "cart.v")
            .unwrap();
        let before = s.signals().points(k).last().unwrap().y;
        s.set_param_key("push.y", 40.0).unwrap();
        s.step().unwrap();
        let after = s.signals().points(k).last().unwrap().y;
        // dv = F/m dt (less the drag): about 0.2 in one 0.01 s step.
        assert!(after - before > 0.15, "{before} -> {after}");
    }

    #[test]
    fn new_input_during_a_replay_starts_a_branch() {
        let mut s = cart(CART);
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        steer(&mut s);
        s.rewind_to(1.0);
        s.run_until(1.5).unwrap();
        // The mass change at 1.2 was replayed; now push the other way.
        s.set_param_key("push.y", -20.0).unwrap();
        assert!(!s.replaying());
        let keys: Vec<(String, f64)> = s
            .recorded()
            .iter()
            .map(|e| (e.key.clone(), e.value))
            .collect();
        assert_eq!(
            keys,
            [
                ("push.y".to_string(), 40.0),
                ("cart.m".to_string(), 4.0),
                ("push.y".to_string(), -20.0)
            ]
        );
        // A restart drops the recording.
        s.restart().unwrap();
        assert!(s.recorded().is_empty());
    }

    #[test]
    fn scheduled_changes_apply_in_every_run_and_bad_ones_are_reported() {
        let scheduled = CART.replace(
            r#""run": {"dt": 0.01}"#,
            r#""run": {"dt": 0.01, "inputs": [
                {"t": 1, "set": "push.y", "to": 30},
                {"t": 0, "set": "cart.v0", "to": 2}]}"#,
        );
        let mut s = cart(&scheduled);
        // The start value at t = 0 set where the run starts.
        let v0 = s.signals().points(1)[0].y;
        assert_eq!(v0, 2.0);
        s.run_until(2.0).unwrap();
        let first = signals(&s);
        assert!(s.recorded().is_empty());
        // Kept by a restart: the second run is the same.
        s.restart().unwrap();
        s.run_until(2.0).unwrap();
        same(&signals(&s), &first, s.signals().names());
        // A change to something that does not exist is a load error.
        let bad = scheduled.replace(r#""set": "push.y""#, r#""set": "push.z""#);
        let err = Session::load("cart", &bad, &registry()).err().unwrap();
        assert!(
            err.contains("run.inputs[") && err.contains("push.z"),
            "{err}"
        );
        // Out of range is noted, not refused.
        let far = scheduled.replace(r#""to": 30"#, r#""to": 300"#);
        let s = cart(&far);
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("outside its suggested range")));
    }
}

mod probe_tests {
    //! Probes besides the implicit one (ADR 0062).
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use crate::probe::{Every, Keep, ProbeSpec};

    fn example(part: &str) -> Session {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    fn spec(name: &str, record: &[&str], every: Every, keep: Keep) -> ProbeSpec {
        ProbeSpec {
            every,
            keep,
            ..ProbeSpec::new(name, record.iter().map(|r| r.to_string()).collect())
        }
    }

    /// `(t, value)` bits of signal `name` in history `h`.
    fn series(h: &History, name: &str) -> Vec<(u64, u64)> {
        let k = h.names().iter().position(|n| n == name).unwrap();
        h.points(k)
            .iter()
            .map(|p| (p.x.to_bits(), p.y.to_bits()))
            .collect()
    }

    /// `flush(false)` says samples are left while a probe's recording is
    /// being read, though the history's could be appended.
    #[test]
    fn flushing_tells_of_a_probes_samples_left() {
        let mut s = example("Cruise control");
        s.record_all_probes();
        s.set_probes(vec![spec(
            "speed",
            &["car.v"],
            Every::Steps(1),
            Keep::default(),
        )])
        .unwrap();
        s.restart().unwrap();
        let history = s.shared_history();
        let probe = s.probe("speed").unwrap();
        let (h, p) = (history.read(), probe.read());
        s.step().unwrap();
        // Both read: nothing can be appended.
        assert!(!s.flush(false));
        drop(h);
        // The history is free, the probe still read.
        assert!(!s.flush(false));
        drop(p);
        assert!(s.flush(false));
        assert_eq!(probe.read().len(), history.read().len());
    }

    #[test]
    fn a_probe_records_a_subset_on_its_own_schedule() {
        // Cruise control, 0.05 s steps; a probe of the speed every 0.5 s.
        let mut s = example("Cruise control");
        // Kept whatever reads them, as a headless run keeps them.
        s.record_all_probes();
        s.set_probes(vec![spec(
            "speed",
            &["car.v"],
            Every::Seconds(0.5),
            Keep::default(),
        )])
        .unwrap();
        assert_eq!(s.probe_names(), ["speed"]);
        s.restart().unwrap();
        s.run_until(5.0).unwrap();
        s.flush(true);
        let probe = s.probe("speed").unwrap();
        let p = probe.read();
        assert_eq!(p.names(), ["car.v"]);
        let times: Vec<f64> = p.times().to_vec();
        assert_eq!(times.len(), 11, "{times:?}");
        // Every sample is the implicit probe's at the same time, exactly.
        let all = s.signals();
        let k = all.names().iter().position(|n| n == "car.v").unwrap();
        for (i, t) in times.iter().enumerate() {
            let j = all.times().iter().position(|x| x == t).unwrap();
            assert_eq!(
                p.value(SignalRef::Signal(0), i).to_bits(),
                all.points(k)[j].y.to_bits()
            );
        }
        drop((p, all));
        // Unknown names are refused, and nothing changes.
        let bad = s.set_probes(vec![spec("x", &["car.nope"], Every::Steps(1), Keep::All)]);
        assert!(bad.unwrap_err()[0].contains("the model has no 'car.nope'"));
        assert_eq!(s.probe_names(), ["speed"]);
    }

    #[test]
    fn probes_keep_the_newest_or_thin_and_replay_after_a_rewind() {
        let mut s = example("Cruise control");
        // Kept whatever reads them, as a headless run keeps them.
        s.record_all_probes();
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        s.set_probes(vec![
            spec(
                "last",
                &["car.v"],
                Every::Steps(1),
                Keep::Last { samples: 20 },
            ),
            spec("every", &["car.v", "cruise.F"], Every::Steps(2), Keep::All),
        ])
        .unwrap();
        s.restart().unwrap();
        s.run_until(10.0).unwrap();
        s.flush(true);
        let last = s.probe("last").unwrap();
        let n = last.read().len();
        assert!((20..=22).contains(&n), "{n}");
        assert!(last.read().times()[n - 1] > 9.9);
        let every = s.probe("every").unwrap();
        let before = series(&every.read(), "cruise.F");
        assert_eq!(before.len(), 101);
        // Rewind to 4 s and run on: the same samples again, bit for bit.
        s.rewind_to(4.0);
        assert!(every.read().times().last().unwrap() <= &4.0);
        s.run_until(10.0).unwrap();
        s.flush(true);
        assert_eq!(series(&every.read(), "cruise.F"), before);
        // Restart: empty, then recording again from t = 0.
        s.restart().unwrap();
        assert_eq!(every.read().len(), 1);
    }

    #[test]
    fn probes_record_grid_fields_with_their_coordinates() {
        let mut s = example("Typed heat equation");
        // Kept whatever reads them, as a headless run keeps them.
        s.record_all_probes();
        s.set_probes(vec![spec(
            "snapshots",
            &["rod.T", "rod.x"],
            Every::Times(vec![0.0, 1.0, 2.0]),
            Keep::All,
        )])
        .unwrap();
        s.restart().unwrap();
        s.run_until(3.0).unwrap();
        s.flush(true);
        let p = s.probe("snapshots").unwrap();
        let h = p.read();
        assert_eq!(h.len(), 3);
        let names: Vec<&str> = h.fields().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["rod.T", "rod.x"]);
        assert_eq!(h.fields()[0].frame(2).len(), 50);
        assert!(h
            .coordinates()
            .iter()
            .any(|c| c.name == "rod.x" && c.of == "rod.T"));
    }

    #[test]
    fn probes_survive_filter_edits_without_what_is_gone() {
        let mut s = example("Cruise control");
        // Kept whatever reads them, as a headless run keeps them.
        s.record_all_probes();
        let filter = FilterEntry::expression("double", "2*car.v", vec!["car.v".into()]);
        s.set_filters(vec![filter]).unwrap();
        s.set_probes(vec![spec(
            "p",
            &["car.v", "double"],
            Every::Steps(1),
            Keep::default(),
        )])
        .unwrap();
        s.set_filters(Vec::new()).unwrap();
        assert_eq!(s.probe_names(), ["p"]);
        assert_eq!(s.probe("p").unwrap().read().names(), ["car.v"]);
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("'double' is no longer recorded")));
    }

    /// The cruise control example with `probes` (JSON) in its controls.
    fn cruise_with(probes: &str) -> String {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(e.text).unwrap();
        v["controls"]["probes"] = serde_json::from_str(probes).unwrap();
        v.to_string()
    }

    #[test]
    fn probes_load_from_the_file_and_save_back() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v"], "every": {"seconds": 0.5},
                 "keep": {"last": 4}, "to": "speed.csv", "canvas": [10, 20]},
                {"name": "bad", "record": ["car.nope"]}]"#,
        );
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        assert_eq!(s.probe_names(), ["speed"]);
        assert!(
            s.warnings
                .iter()
                .any(|w| w.contains("controls.probes[1]") && w.contains("car.nope")),
            "{:?}",
            s.warnings
        );
        let spec = s.probe_specs()[0].clone();
        assert_eq!(spec.every, Every::Seconds(0.5));
        assert_eq!(spec.keep, Keep::Last { samples: 4 });
        assert_eq!(spec.canvas, Some([10.0, 20.0]));
        let saved = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &saved, &registry()).unwrap();
        assert_eq!(again.probe_specs(), [&spec]);
    }

    #[test]
    fn a_headless_run_records_what_the_app_does() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v"], "every": {"steps": 10}, "keep": {"all": true},
                 "to": "speed.csv"}]"#,
        );
        let r = crate::run::run_probes(
            &text,
            &registry(),
            Some(5.0),
            None,
            Vec::new(),
            &[],
            None,
            None,
        )
        .unwrap();
        let (spec, h) = &r.probes[0];
        assert_eq!(spec.name, "speed");
        let h = h.read();
        assert_eq!(h.len(), 11);
        // Every sample is the plain run's at the same step, exactly.
        let plain = crate::run::run(&text, &registry(), Some(5.0), None).unwrap();
        let v = &plain
            .outputs
            .iter()
            .find(|o| o.name == "car.v")
            .unwrap()
            .values;
        for (k, p) in h.points(0).iter().enumerate() {
            assert_eq!(p.y.to_bits(), v[10 * k].to_bits());
        }
        let err = crate::run::run_probes(
            &text,
            &registry(),
            None,
            None,
            Vec::new(),
            &["x".into()],
            None,
            None,
        );
        assert!(err.is_err_and(|e| e.contains("no probe 'x'")));
    }

    #[test]
    fn probe_edits_keep_what_unchanged_probes_recorded() {
        let text =
            cruise_with(r#"[{"name": "speed", "record": ["car.v"], "keep": {"all": true}}]"#);
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        // Kept whatever reads them, as a headless run keeps them.
        s.record_all_probes();
        s.restart().unwrap();
        s.run_until(1.0).unwrap();
        s.flush(true);
        let before = s.probe("speed").unwrap().read().len();
        assert_eq!(before, 21);
        // Moved, and a probe added that records nothing yet.
        let mut entries = s.probe_entries();
        entries[0].canvas = Some([5.0, 5.0]);
        entries.push(ProbeSpec::new("later", vec![]).to_entry());
        entries[1].every = Some(simsandbox_file::ProbeEvery {
            seconds: Some(0.5),
            ..Default::default()
        });
        entries[1].record = vec!["car.v".into()];
        s.set_probe_entries(&entries).unwrap();
        s.run_until(2.0).unwrap();
        s.flush(true);
        assert_eq!(s.probe("speed").unwrap().read().len(), 41);
        // The new one from now on, at multiples of 0.5 s after t = 1.
        let later: Vec<f64> = s.probe("later").unwrap().read().times().to_vec();
        assert_eq!(later.len(), 2, "{later:?}");
        assert!((later[0] - 1.5).abs() < 1e-9);
        assert_eq!(s.probe_entries()[0].canvas, Some([5.0, 5.0]));
        // A probe that records nothing is kept.
        entries[1].record.clear();
        s.set_probe_entries(&entries).unwrap();
        assert_eq!(s.probe_names(), ["speed", "later"]);
    }

    #[test]
    fn the_history_records_only_what_is_wired_and_probes_anything() {
        // Cruise control's views read car.v and cruise.F, and so on.
        let mut s = example("Cruise control");
        // Kept whatever reads them, as a headless run keeps them.
        s.record_all_probes();
        s.set_view_reads(vec!["car.v".into(), "probe:speed/cruise.F".into()]);
        assert_eq!(s.signals().names(), ["car.v"]);
        // Everything can still be bound and probed.
        let h = s.signals();
        assert!(s.available(&h).scalars.contains(&"cruise.I".to_string()));
        drop(h);
        s.set_probes(vec![spec("i", &["cruise.I"], Every::Steps(1), Keep::All)])
            .unwrap();
        s.run_until(1.0).unwrap();
        s.flush(true);
        // Added after the sample at t = 0: every step since.
        assert_eq!(s.probe("i").unwrap().read().len(), 20);
        // Wiring cruise.I to a view records it from now; car.v keeps its past.
        s.set_view_reads(vec!["car.v".into(), "cruise.I".into()]);
        s.run_until(2.0).unwrap();
        s.flush(true);
        let h = s.signals();
        assert_eq!(h.names(), ["car.v", "cruise.I"]);
        let i = h.values(h.resolve("cruise.I").unwrap());
        assert_eq!(h.len(), 41);
        // The sample at the time it was wired is had from the model then.
        assert!(i[..20].iter().all(|v| v.is_nan()));
        assert!(i[20..].iter().all(|v| v.is_finite()));
        assert!(s.has_gaps());
        assert!(h
            .values(h.resolve("car.v").unwrap())
            .iter()
            .all(|v| v.is_finite()));
    }

    /// The values of `name` in a filter's own recording, with its times.
    fn derived_series(s: &Session, filter: &str, name: &str) -> (Vec<f64>, Vec<f64>) {
        let (_, h) = s
            .derived_histories()
            .into_iter()
            .find(|(n, _)| n == filter)
            .unwrap();
        let h = h.read();
        let r = h.resolve(name).unwrap();
        (h.times().to_vec(), h.values(r))
    }

    #[test]
    fn filters_on_a_probe_run_on_its_samples() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v", "cruise.F"], "every": {"seconds": 0.5},
                 "keep": {"all": true}}]"#,
        );
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        let mut filters = s.filters().to_vec();
        let mut power = FilterEntry::expression("power", "car.v*cruise.F", Vec::new());
        power.probe = Some("speed".into());
        filters.push(power);
        s.set_filters(filters.clone()).unwrap();
        s.run_until(2.0).unwrap();
        s.flush(true);
        // Every 0.5 s, from the probe's samples, in a recording of its own.
        let (t, w) = derived_series(&s, "power", "power");
        let p = s.probe("speed").unwrap();
        let h = p.read();
        assert_eq!(t, h.times());
        let (v, f) = (
            h.values(SignalRef::Signal(0)),
            h.values(SignalRef::Signal(1)),
        );
        assert!(w
            .iter()
            .zip(v.iter().zip(&f))
            .all(|(w, (v, f))| (w - v * f).abs() < 1e-9));
        drop(h);
        assert!(s.signals().resolve("power").is_none());
        assert!(s.derived_names().contains(&"power".to_string()));
        let saved = s.save_json(1.0, None).unwrap();
        assert!(saved.contains("\"probe\": \"speed\""));
        // A filter on a probe reads only what the probe records.
        let mut bad = FilterEntry::expression("pi", "cruise.I*2", Vec::new());
        bad.probe = Some("speed".into());
        filters.push(bad);
        s.set_filters(filters.clone()).unwrap();
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("pi") && w.contains("does not record")));
        let mut nowhere = FilterEntry::expression("x", "1", Vec::new());
        nowhere.probe = Some("gone".into());
        s.set_filters(vec![nowhere]).unwrap();
        assert!(s.warnings.iter().any(|w| w.contains("no probe 'gone'")));
    }

    #[test]
    fn filters_on_several_probes_run_paired_or_on_any() {
        // car.v every 0.2 s, cruise.F every 0.3 s (steps of 0.05 s).
        let text = cruise_with(
            r#"[{"name": "a", "record": ["car.v"], "every": {"steps": 4}, "keep": {"all": true}},
                {"name": "b", "record": ["cruise.F"], "every": {"steps": 6}, "keep": {"all": true}}]"#,
        );
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        // Rewinding needs a recorder (ADR 0064).
        s.set_recorder(Some(Default::default()));
        let both = |name: &str, when: Option<&str>| {
            let mut f = FilterEntry::expression(name, "car.v*cruise.F", Vec::new());
            f.from = [
                ("car.v".into(), "a".into()),
                ("cruise.F".into(), "b".into()),
            ]
            .into();
            f.when = when.map(String::from);
            f
        };
        // And a filter reading a filter: on its clock.
        let half = FilterEntry::expression("half", "paired/2", Vec::new());
        s.set_filters(vec![
            both("paired", None),
            both("latest", Some("any")),
            half,
        ])
        .unwrap();
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        s.run_until(1.2).unwrap();
        s.flush(true);
        let step =
            |t: &[f64]| -> Vec<i64> { t.iter().map(|t| (t / 0.05).round() as i64).collect() };
        // Paired: where both recorded, every 12 steps.
        let (t, w) = derived_series(&s, "paired", "paired");
        assert_eq!(step(&t), [12, 24]);
        let (th, h) = derived_series(&s, "half", "half");
        assert_eq!(th, t);
        assert!(h.iter().zip(&w).all(|(h, w)| (2.0 * h - w).abs() < 1e-9));
        // Any: whenever either recorded (steps 4, 6, 8, 12, ...), with the
        // other's latest value; NaN until both have one.
        let (t, w) = derived_series(&s, "latest", "latest");
        assert_eq!(step(&t), [4, 6, 8, 12, 16, 18, 20, 24]);
        assert!(w[0].is_nan() && w[1].is_finite());
        let a = s.probe("a").unwrap();
        let b = s.probe("b").unwrap();
        let (a, b) = (a.read(), b.read());
        // At step 8: car.v at 8, cruise.F held from 6.
        let (v8, f6) = (
            a.values(SignalRef::Signal(0))[1],
            b.values(SignalRef::Signal(0))[0],
        );
        assert!((w[2] - v8 * f6).abs() < 1e-9);
        drop((a, b));
        // After a rewind it records the same again.
        let before = derived_series(&s, "latest", "latest").1;
        s.rewind_to(0.5);
        s.run_until(1.2).unwrap();
        s.flush(true);
        let again = derived_series(&s, "latest", "latest").1;
        assert_eq!(
            before.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            again.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn snapshots_leave_out_outputs_had_again_from_state_and_restore_them_exactly() {
        let all_outputs = |s: &Session| -> Vec<u64> {
            let mut v = Vec::new();
            for e in &s.flat.blocks {
                let id = s.ids[&e.name];
                let spec = s.sim.model().block(id).unwrap().spec().clone();
                for p in spec.outputs() {
                    v.extend(
                        s.sim
                            .output(id, &p.name)
                            .unwrap_or(&[])
                            .iter()
                            .map(|x| x.to_bits()),
                    );
                }
            }
            v
        };
        for e in EXAMPLES {
            let mut s =
                Session::load_in(e.name, e.text, &registry(), e.folder().as_deref()).unwrap();
            for _ in 0..20 {
                s.step().unwrap();
            }
            let before = all_outputs(&s);
            let snap = s.sim.snapshot();
            for _ in 0..7 {
                s.step().unwrap();
            }
            s.sim.restore(&snap);
            assert_eq!(all_outputs(&s), before, "{}", e.name);
        }
        // Gray–Scott: its outputs (u, v, reaction, the axes) come from its
        // state and parameters alone: a snapshot holds the two state grids.
        let gs = EXAMPLES.iter().find(|e| e.name.contains("Gray")).unwrap();
        let s = Session::load(gs.name, gs.text, &registry()).unwrap();
        let snap = s.sim.snapshot();
        let cells = 128 * 128;
        assert!(snap.values() < 2 * cells + 100, "{} values", snap.values());
    }

    #[test]
    fn rewind_needs_a_recorder_and_reaches_back_to_when_it_was_added() {
        let mut s = example("Cruise control");
        assert!(s.recorder().is_none());
        s.run_until(1.0).unwrap();
        // No recorder: no snapshots, nothing to rewind to.
        assert_eq!(s.history_range(), (s.time(), s.time()));
        s.rewind_to(0.5);
        assert!((s.time() - 1.0).abs() < 1e-9);
        // Added at t = 1: rewind reaches back to then, not before.
        s.set_recorder(Some(simsandbox_file::RecorderEntry {
            every: Some(4),
            ..Default::default()
        }));
        s.run_until(2.0).unwrap();
        let (first, last) = s.history_range();
        assert!((first - 1.0).abs() < 1e-9 && (last - 2.0).abs() < 1e-9);
        s.rewind_to(0.5);
        assert!((s.time() - 1.0).abs() < 1e-9);
        // Saved with the model; removed, its snapshots go.
        assert!(s.save_json(1.0, None).unwrap().contains("\"recorder\""));
        s.set_recorder(None);
        assert_eq!(s.history_range(), (s.time(), s.time()));
        assert!(!s.save_json(1.0, None).unwrap().contains("\"recorder\""));
    }

    /// Cruise control whose controller reads the speed through probe
    /// `sampled`, every `every` seconds (ADR 0063 step 3).
    fn sampled_cruise(every: f64) -> String {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(e.text).unwrap();
        v["controls"]["probes"] = serde_json::json!([
            {"name": "sampled", "record": ["car.v"], "every": {"seconds": every}}
        ]);
        v["connections"][0]["from"] = "probe:sampled.car.v".into();
        v.to_string()
    }

    #[test]
    fn a_probe_feeding_the_model_holds_its_samples_with_no_delay() {
        let text = sampled_cruise(1.0);
        let r = crate::run::run(&text, &registry(), Some(5.0), None).unwrap();
        let series = |name: &str| -> &Vec<f64> {
            &r.outputs.iter().find(|o| o.name == name).unwrap().values
        };
        let (v, e) = (series("car.v"), series("cruise.e"));
        // 0.05 s steps: samples at steps 0, 20, 40, ... The error uses the
        // speed of that very step (no delay) and holds it until the next.
        for (k, e) in e.iter().enumerate() {
            let sampled = 20 * (k / 20);
            assert_eq!(e.to_bits(), (25.0 - v[sampled]).to_bits(), "step {k}");
        }
        // Sampled every step, it is a direct wire between blocks solved
        // apart (a hold never joins a fused group), bit for bit.
        let direct_text = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap()
            .text;
        let apart = |text: &str| {
            let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
            v["fusion"] = "off".into();
            v.to_string()
        };
        let every_step = apart(&sampled_cruise(1e-9));
        let a = crate::run::run(&apart(direct_text), &registry(), Some(5.0), None).unwrap();
        let b = crate::run::run(&every_step, &registry(), Some(5.0), None).unwrap();
        let bits = |r: &crate::run::Results| -> Vec<u64> {
            r.outputs
                .iter()
                .filter(|o| !o.name.starts_with("probe:"))
                .flat_map(|o| o.values.iter().map(|x| x.to_bits()))
                .collect()
        };
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn a_probe_file_read_back_drives_the_model_as_the_probe_did() {
        // ADR 0063 step 4: the controller reads the speed every second
        // from a probe; the probe's file, read back by an input.file block
        // in place of the probe, gives the same run bit for bit.
        let dir = temp_dir("read-back");
        let mut live: serde_json::Value = serde_json::from_str(&sampled_cruise(1.0)).unwrap();
        live["controls"]["probes"][0]["to"] = "speed.csv".into();
        live["controls"]["probes"][0]["stream"] = true.into();
        let watched = |s: &mut Session| {
            s.set_view_reads(vec!["car.v".into(), "cruise.e".into()]);
            s.run_until(5.0).unwrap();
            s.flush(true);
            let h = s.signals();
            ["car.v", "cruise.e"]
                .iter()
                .flat_map(|n| h.values(h.resolve(n).unwrap()))
                .map(f64::to_bits)
                .collect::<Vec<_>>()
        };
        let mut a = Session::load("live", &live.to_string(), &registry()).unwrap();
        a.set_probe_dir(Some(dir.clone()));
        a.restart().unwrap();
        let first = watched(&mut a);
        drop(a);
        let mut read: serde_json::Value = serde_json::from_str(&sampled_cruise(1.0)).unwrap();
        read["controls"]["probes"] = serde_json::json!([]);
        read["blocks"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "recorded", "type": "input.file",
                "settings": {"file": "speed.csv", "unit.car.v": "m/s"}
            }));
        read["connections"][0]["from"] = "recorded.car_v".into();
        let text = read.to_string();
        // A relative file needs the model's folder.
        let e = Session::load("read", &text, &registry()).err().unwrap();
        assert!(e.contains("save the model first"), "{e}");
        let mut b = Session::load_in("read", &text, &registry(), Some(&dir)).unwrap();
        assert_eq!(watched(&mut b), first);
        // The run hash follows the file's contents.
        let hash = |s: &Session| s.run_file("m.json", "", "", "").model_hash;
        let before = hash(&b);
        let csv = std::fs::read_to_string(dir.join("speed.csv")).unwrap();
        std::fs::write(dir.join("speed.csv"), csv.replacen("\n0,", "\n0.0,", 1)).unwrap();
        let b = Session::load_in("read", &text, &registry(), Some(&dir)).unwrap();
        assert_ne!(hash(&b), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_held_probe_rewinds_and_replays_exactly() {
        let mut s = Session::load("c", &sampled_cruise(0.7), &registry()).unwrap();
        s.set_recorder(Some(Default::default()));
        s.set_view_reads(vec!["cruise.e".into()]);
        s.run_until(5.0).unwrap();
        s.flush(true);
        let e = |s: &Session| {
            let h = s.signals();
            h.values(h.resolve("cruise.e").unwrap())
        };
        let first = e(&s);
        s.rewind_to(2.2);
        s.run_until(5.0).unwrap();
        s.flush(true);
        let again = e(&s);
        assert_eq!(
            first.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            again.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
        // The hold is not a recorded output of the model.
        assert!(!s.bindable_scalars().iter().any(|n| n.starts_with("probe:")));
    }

    #[test]
    fn recompute_fills_what_was_wired_later_bit_for_bit() {
        // ADR 0062/0063: wired mid-run, a signal and a field have no past;
        // recomputing on demand fills it as if it had been recorded from
        // the start, live input included, and leaves the model as it was.
        let example = |name: &str| {
            EXAMPLES
                .iter()
                .find(|e| e.name.contains(name))
                .unwrap()
                .text
        };
        let run = |text: &str, (key, value): (&str, f64), reads: &[&str], late: bool| {
            let mut s = Session::load("m", text, &registry()).unwrap();
            s.set_recorder(Some(Default::default()));
            let all: Vec<String> = reads.iter().map(|r| r.to_string()).collect();
            s.set_view_reads(if late { all[..1].to_vec() } else { all.clone() });
            s.run_until(2.0).unwrap();
            s.set_param_key(key, value).unwrap();
            s.run_until(4.0).unwrap();
            if late {
                s.set_view_reads(all);
            }
            s.flush(true);
            s
        };
        let bits = |s: &Session, name: &str| -> Vec<u64> {
            let h = s.signals();
            if let Some(f) = h.fields().iter().find(|f| f.name == name) {
                return (0..h.len())
                    .flat_map(|k| f.frame(k).to_vec())
                    .map(f64::to_bits)
                    .collect();
            }
            h.values(h.resolve(name).unwrap())
                .iter()
                .map(|x| x.to_bits())
                .collect()
        };
        for (text, key, reads) in [
            (
                example("Cruise control"),
                ("cruise.kp", 600.0),
                ["car.v", "cruise.e"],
            ),
            (
                example("Heat"),
                ("rod.alpha", 0.02),
                ["rod.position", "rod.temperature"],
            ),
        ] {
            let mut whole = run(text, key, &reads, false);
            let mut late = run(text, key, &reads, true);
            assert!(!whole.has_gaps());
            assert!(late.has_gaps());
            assert_ne!(bits(&late, reads[1]), bits(&whole, reads[1]));
            late.recompute().unwrap();
            assert!(!late.has_gaps());
            for r in reads {
                assert_eq!(bits(&late, r), bits(&whole, r), "{r}");
            }
            // Where it was, as it was: running on gives the same.
            assert_eq!(late.time().to_bits(), whole.time().to_bits());
            whole.run_until(5.0).unwrap();
            late.run_until(5.0).unwrap();
            whole.flush(true);
            late.flush(true);
            for r in reads {
                assert_eq!(bits(&late, r), bits(&whole, r), "{r}");
            }
            // Without the recorder's snapshots, there is nothing to run from.
            let mut bare = run(text, key, &reads, true);
            bare.set_recorder(None);
            assert!(bare.has_gaps());
            assert!(bare.recompute().unwrap_err().contains("snapshots"));
        }
    }

    /// A fresh folder for files a test writes.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("simsandbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_probe_writes_its_file_as_it_records_and_rewinds_cut_it_back() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v"], "keep": {"last": 2},
                 "to": "speed.csv", "stream": true}]"#,
        );
        let dir = temp_dir("stream");
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        s.set_recorder(Some(Default::default()));
        s.set_probe_dir(Some(dir.clone()));
        s.restart().unwrap();
        s.run_until(2.0).unwrap();
        s.flush(true);
        let csv = || std::fs::read_to_string(dir.join("speed.csv")).unwrap();
        // Every sample in the file, whatever Keep keeps in memory (here
        // nothing: no view reads it).
        let all = csv();
        assert_eq!(all.lines().count(), 1 + 41, "{all}");
        assert!(s.probe("speed").unwrap().read().is_empty());
        // A rewind cuts the file back; running on writes the same again.
        s.rewind_to(1.01);
        s.flush(true);
        assert_eq!(csv().lines().count(), 1 + 21);
        s.run_until(2.0).unwrap();
        s.flush(true);
        assert_eq!(csv(), all);
        // A restart starts it afresh.
        s.restart().unwrap();
        s.flush(true);
        assert_eq!(csv().lines().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_streamed_grid_is_a_npy_file_numpy_reads() {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Typed heat"))
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(e.text).unwrap();
        let rod = v["blocks"][0]["name"].as_str().unwrap().to_string();
        v["controls"]["probes"] = serde_json::json!([{
            "name": "profile", "record": [format!("{rod}.T")], "every": {"steps": 5},
            "to": "rod.csv", "stream": true
        }]);
        let dir = temp_dir("stream-npy");
        let mut s = Session::load("rod", &v.to_string(), &registry()).unwrap();
        s.set_probe_dir(Some(dir.clone()));
        s.restart().unwrap();
        for _ in 0..20 {
            s.step().unwrap();
        }
        s.flush(true);
        let npy = std::fs::read(dir.join(format!("rod.{rod}.T.npy"))).unwrap();
        let len = u16::from_le_bytes([npy[8], npy[9]]) as usize;
        let header = std::str::from_utf8(&npy[10..10 + len]).unwrap();
        // Steps 0, 5, 10, 15, 20: five frames of one row.
        assert!(header.contains("'shape': (5, 1, "), "{header}");
        let cells: usize = header
            .split(", ")
            .nth(4)
            .unwrap()
            .trim_end_matches(')')
            .parse()
            .unwrap_or(0);
        assert_eq!(npy.len(), 10 + len + 5 * cells * 8, "{header}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn probes_keep_samples_in_memory_only_when_something_reads_them() {
        // Feeding the controller only: nothing kept.
        let mut s = Session::load("c", &sampled_cruise(0.5), &registry()).unwrap();
        s.run_until(2.0).unwrap();
        s.flush(true);
        assert!(s.probe("sampled").unwrap().read().is_empty());
        // A view reading it: kept from then on.
        s.set_view_reads(vec!["probe:sampled/car.v".into()]);
        s.run_until(4.0).unwrap();
        s.flush(true);
        assert_eq!(s.probe("sampled").unwrap().read().len(), 4);
        // A headless run keeps every probe.
        let r = crate::run::run_probes(
            &sampled_cruise(0.5),
            &registry(),
            Some(2.0),
            None,
            Vec::new(),
            &[],
            None,
            None,
        )
        .unwrap();
        assert_eq!(r.probes[0].1.read().len(), 5);
    }
}

mod pointer_tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    /// ADR 0093 point 6: a model file made before `picked` (the drag
    /// example) reads it as -1, never picking, and is written back without
    /// it; its `input.pointer` blocks are those a scene may grab with.
    #[test]
    fn an_older_file_never_picks_and_is_saved_unchanged() {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Drag the bob"))
            .unwrap();
        assert!(!e.text.contains("picked"));
        let mut s = Session::load(e.name, e.text, &registry()).unwrap();
        let picked = s
            .params()
            .into_iter()
            .find(|p| p.key == "hand.picked")
            .unwrap();
        assert_eq!(picked.value, -1.0);
        assert_eq!(s.pointers(), ["hand"]);
        // Its recorded run (x, y and down) leaves picked at -1.
        let track = [
            (0.1, "hand.x", 1.0),
            (0.1, "hand.down", 1.0),
            (0.3, "hand.down", 0.0),
        ]
        .map(|(t, key, value)| InputEvent {
            t,
            key: key.into(),
            value,
        })
        .to_vec();
        s.replay(track).unwrap();
        s.run_until(0.5).unwrap();
        let id = s.ids["hand"];
        assert_eq!(s.sim.output(id, "picked"), Some(&[-1.0][..]));
        // The run's starting point names it, as it names every value.
        let start = &s.start_file().blocks[0].params;
        assert_eq!(
            start.get("picked"),
            Some(&simsandbox_file::ParamValue::Number(-1.0))
        );
        // The file itself is written back as it was.
        let (f, _) = ModelFile::from_json(e.text).unwrap();
        assert_eq!(f.to_json(), e.text);
    }
}
