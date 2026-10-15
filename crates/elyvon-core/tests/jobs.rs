use elyvon_core::jobs::{JobSnapshot, JobState};

#[test]
fn v1_wire_fixture_keeps_every_state_and_unknown_totals() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/jobs-v1.json")).unwrap();
    let jobs: Vec<JobSnapshot> = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&jobs).unwrap(), value);
    for job in jobs {
        job.validate().unwrap();
    }
}

#[test]
fn jobs_reject_unknown_versions_bad_ids_and_progress_beyond_total() {
    let mut job = JobSnapshot {
        contract_version: 1,
        id: "task".into(),
        state: JobState::Running {
            completed: 11,
            total: Some(10),
        },
    };
    assert!(job.validate().is_err());
    job.state = JobState::Completed;
    job.contract_version = 2;
    assert!(job.validate().is_err());
    job.contract_version = 1;
    for id in ["".to_owned(), "x".repeat(129), "bad\n".into()] {
        job.id = id;
        assert!(job.validate().is_err());
    }
}
