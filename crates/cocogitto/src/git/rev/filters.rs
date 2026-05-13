use crate::git::{error::Git2Error, repository::Repository};
use crate::settings::MonoRepoPackage;
use crate::SETTINGS;

use git2::Commit;
use globset::{Candidate, Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use once_cell::sync::Lazy;
use std::{collections::HashMap, path::Path};

#[derive(Debug)]
struct PackagePathFilter {
    include: GlobSet,
    exclude: GlobSet,
}

impl PackagePathFilter {
    fn from_package(package: &MonoRepoPackage) -> Self {
        Self::new(
            package.path.to_str().expect("valid package path"),
            &package.include,
            &package.ignore,
        )
    }

    fn is_match<P: AsRef<Path> + ?Sized>(&self, path: &P) -> bool {
        let candidate = Candidate::new(path);
        self.include.is_match_candidate(&candidate) && !self.exclude.is_match_candidate(&candidate)
    }

    fn new(package_path: &str, include_paths: &[String], exclude_paths: &[String]) -> Self {
        let include = {
            let mut builder = GlobSetBuilder::new();
            builder.add(Self::build_glob(package_path, true));
            for include in include_paths {
                builder.add(Self::build_glob(include, false));
            }
            builder.build().expect("valid globset")
        };
        let exclude = {
            let mut builder = GlobSetBuilder::new();
            for exclude in exclude_paths {
                builder.add(Self::build_glob(exclude, false));
            }
            builder.build().expect("valid globset")
        };

        PackagePathFilter { include, exclude }
    }

    fn build_glob(pattern: &str, is_dir: bool) -> Glob {
        // https://github.com/cocogitto/cocogitto/issues/544
        let glob = pattern.strip_prefix("./").unwrap_or(pattern);
        // https://github.com/cocogitto/cocogitto/issues/462
        let glob = glob.strip_suffix("/").unwrap_or(glob);
        let glob = if is_dir || pattern.ends_with("/") {
            format!("{glob}/**")
        } else {
            glob.to_string()
        };
        GlobBuilder::new(&glob)
            .literal_separator(true)
            .build()
            .expect("glob should be valid")
    }
}

static PATH_FILTER: Lazy<HashMap<String, PackagePathFilter>> = Lazy::new(|| {
    SETTINGS
        .list_packages()
        .iter()
        .map(|(name, package)| (name.clone(), PackagePathFilter::from_package(package)))
        .collect()
});

impl Repository {
    pub fn is_commit_global(&self, commit: &Commit<'_>) -> Result<bool, Git2Error> {
        if commit.parent(0).is_err() {
            return Ok(true);
        }
        self.check_diff_paths(commit, |path| {
            PATH_FILTER.values().all(|filter| !filter.is_match(path))
        })
    }

    pub fn is_commit_in_package(
        &self,
        commit: &Commit<'_>,
        package: &str,
    ) -> Result<bool, Git2Error> {
        let filter = PATH_FILTER.get(package).expect("invalid package");
        self.check_diff_paths(commit, |path| filter.is_match(path))
    }

    fn check_diff_paths(
        &'_ self,
        commit: &Commit<'_>,
        predicate: impl Fn(&Path) -> bool,
    ) -> Result<bool, Git2Error> {
        let parent = commit
            .parent(0)
            .ok()
            .map(|commit| commit.tree())
            .transpose()?;
        let current = commit.tree()?;
        let diff = self
            .0
            .diff_tree_to_tree(parent.as_ref(), Some(&current), None)?;
        Ok(diff.deltas().any(|delta| {
            delta.old_file().path().is_some_and(&predicate)
                || delta.new_file().path().is_some_and(&predicate)
        }))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn package_filter_basic() {
        let filter = PackagePathFilter::new("dir", &[], &[]);
        assert!(filter.is_match("dir/file"));
    }

    #[test]
    fn package_filter_include_file() {
        let filter = PackagePathFilter::new("dir", &["file".to_string()], &[]);
        assert!(filter.is_match("file"))
    }

    #[test]
    fn package_filter_include_dir() {
        let filter = PackagePathFilter::new("dir", &["other/**".to_string()], &[]);
        assert!(filter.is_match("other/file"))
    }

    #[test]
    fn package_filter_exclude_file() {
        let filter = PackagePathFilter::new("dir", &[], &["dir/file".to_string()]);
        assert!(!filter.is_match("dir/file"))
    }

    #[test]
    fn package_filter_exclude_dir() {
        let filter = PackagePathFilter::new("dir", &[], &["dir/test/**".to_string()]);
        assert!(!filter.is_match("dir/test/file"))
    }

    #[test]
    fn package_filter_trailing_slash() {
        let filter = PackagePathFilter::new("dir/", &[], &[]);
        assert!(filter.is_match("dir/file"));
    }

    #[test]
    fn package_filter_include_trailing_slash() {
        let filter = PackagePathFilter::new("dir", &["other/".to_string()], &[]);
        assert!(filter.is_match("other/file"));
    }

    #[test]
    fn package_filter_exclude_trailing_slash() {
        let filter = PackagePathFilter::new("dir", &[], &["dir/test/".to_string()]);
        assert!(!filter.is_match("dir/test/file"));
    }

    #[test]
    fn package_filter_leading_dotslash() {
        let filter = PackagePathFilter::new("./dir", &[], &[]);
        assert!(filter.is_match("dir/file"));
    }

    #[test]
    fn package_filter_include_leading_dotslash() {
        let filter = PackagePathFilter::new("dir", &["./other/**".to_string()], &[]);
        assert!(filter.is_match("other/file"));
    }

    #[test]
    fn package_filter_exclude_leading_dotslash() {
        let filter = PackagePathFilter::new("dir", &[], &["./dir/test/**".to_string()]);
        assert!(!filter.is_match("dir/test/file"));
    }
}
