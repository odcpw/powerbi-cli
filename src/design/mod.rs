//! Data-driven design-system primitives.
//!
//! The design system deliberately keeps layout policy separate from PBIR
//! mutation code.  The grid resolver is pure: given a template, a page size,
//! and grid settings it returns stable slot coordinates or a pointer-rich
//! diagnostic.  Consumers such as `report layout auto` are responsible for
//! applying those coordinates to a project.

pub(crate) mod grid;
