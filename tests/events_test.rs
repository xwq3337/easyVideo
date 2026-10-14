use easydownvideo::core::events::ProgressHandle;

#[test]
fn progress_handle_tracks_counts() {
    let p = ProgressHandle::new();
    assert_eq!(p.snapshot(), (0, 0));
    p.set_total(1000);
    p.add(250);
    p.add(250);
    assert_eq!(p.snapshot(), (500, 1000));
    p.reset();
    assert_eq!(p.snapshot(), (0, 1000));
}

#[test]
fn progress_handle_unit_defaults_to_bytes() {
    use easydownvideo::core::events::ProgressUnit;
    let p = ProgressHandle::new();
    assert_eq!(p.unit(), ProgressUnit::Bytes);
    p.set_unit(ProgressUnit::Segments);
    assert_eq!(p.unit(), ProgressUnit::Segments);
}

#[test]
fn sub_rolls_back_without_underflow() {
    let p = ProgressHandle::new();
    p.set_total(1000);
    p.add(300);
    p.sub(120);
    assert_eq!(p.snapshot(), (180, 1000));
    // 回滚量超过已计入时饱和到 0，绝不回绕成天文数字
    p.sub(999);
    assert_eq!(p.snapshot(), (0, 1000));
}

#[test]
fn task_state_variants_exist() {
    use easydownvideo::core::events::TaskState;
    let states = [
        TaskState::Queued,
        TaskState::Probing,
        TaskState::AwaitingChoice,
        TaskState::Downloading,
        TaskState::Merging,
        TaskState::Paused,
        TaskState::Done,
        TaskState::Failed("x".into()),
        TaskState::Canceled,
    ];
    assert_eq!(states.len(), 9);
}
