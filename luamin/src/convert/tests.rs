use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tracked(Rc<RefCell<usize>>);
impl Drop for Tracked {
    fn drop(&mut self) {
        *self.0.borrow_mut() += 1;
    }
}

#[test]
fn array_conversion_drops_initialized_prefix_on_failure() {
    let drops = Rc::new(RefCell::new(0));
    let input = vec![Some(Tracked(drops.clone())), None];
    assert!(ArrayHandler::<Tracked, 2>::handle_iter(input.into_iter()).is_none());
    assert_eq!(*drops.borrow(), 1);
}

#[test]
fn array_conversion_drops_initialized_prefix_on_panic() {
    let drops = Rc::new(RefCell::new(0));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let iter = (0..2).map(|index| {
            if index == 1 {
                panic!("conversion panic");
            }
            Some(Tracked(drops.clone()))
        });
        let _ = ArrayHandler::<Tracked, 2>::handle_iter(iter);
    }));
    assert!(result.is_err());
    assert_eq!(*drops.borrow(), 1);
}

#[test]
fn array_conversion_handles_empty_arrays_and_rejects_wrong_lengths() {
    assert_eq!(
        ArrayHandler::<i32, 0>::handle_iter(std::iter::empty()),
        Some([])
    );
    assert!(ArrayHandler::<i32, 1>::handle_iter(std::iter::empty()).is_none());
    assert_eq!(
        ArrayHandler::<i32, 2>::handle_iter([Some(1), Some(2)].into_iter()),
        Some([1, 2])
    );
}
