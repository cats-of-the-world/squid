use anyhow::Result;
use anyhow::{anyhow, Context};
use std::fs;
use std::fs::ReadDir;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[derive(Debug)]
pub struct LazyFolderReader {
    files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub name: String,
    pub contents: String,
    pub path: PathBuf,
}

impl TemplateFile {
    pub(crate) fn new(path: &PathBuf) -> Result<Self> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(e) => return Err(e).context("could not read file"),
        };

        Ok(TemplateFile {
            name: path
                .file_name()
                .ok_or(anyhow!("could not get the file name"))?
                .to_str()
                .ok_or(anyhow!("could not transform file name into str"))?
                .to_string(),
            contents,
            path: path.to_path_buf(),
        })
    }
}

pub struct LazyFolderReaderIterator<'a> {
    reader: &'a LazyFolderReader,
    current_position: usize,
}

impl Iterator for LazyFolderReaderIterator<'_> {
    type Item = Result<TemplateFile>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        if self.current_position == self.reader.files.len() {
            return None;
        }

        let name = self.reader.files.get(self.current_position)?;
        self.current_position += 1;

        Some(TemplateFile::new(name))
    }
}

impl Iterator for LazyFolderReader {
    type Item = Result<TemplateFile>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        if self.files.is_empty() {
            return None;
        }

        let current = self.files.pop().expect("files is non-empty, checked above");
        Some(TemplateFile::new(&current))
    }
}

impl LazyFolderReader {
    pub fn new(dir: &Path, extension: &str) -> Result<Self> {
        let paths = fs::read_dir(dir).context("could not read the folder")?;

        let files = Self::scan(paths, extension)?;

        Ok(Self { files })
    }

    pub async fn async_next(&mut self) -> Option<Result<TemplateFile>> {
        if self.files.is_empty() {
            return None;
        }

        let current = self.files.pop().expect("files is non-empty, checked above");
        Some(TemplateFile::new(&current))
    }

    fn scan(paths: ReadDir, extension: &str) -> Result<Vec<PathBuf>> {
        let paths: Vec<PathBuf> = paths
            .map(|entry| entry.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .context("failed to read directory entry")?;

        let mut files: Vec<PathBuf> = paths
            .iter()
            .filter(|path| path.is_file())
            .filter(|path| {
                if let Some(e) = path.extension() {
                    e.eq(extension)
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        let sub_directories: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| path.is_dir())
            .filter_map(|dir| {
                fs::read_dir(&dir)
                    .ok()
                    .and_then(|rd| Self::scan(rd, extension).ok())
            })
            .flatten()
            .collect();

        files.extend(sub_directories);

        Ok(files)
    }
}

impl<'a> IntoIterator for &'a LazyFolderReader {
    type Item = Result<TemplateFile>;
    type IntoIter = LazyFolderReaderIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter {
            reader: self,
            current_position: 0,
        }
    }
}

pub(crate) async fn write_to_disk(dir: PathBuf, file_name: &str, output: String) -> Result<()> {
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("failed to create directory '{}'", dir.display()))?;
    let output_file = dir.join(file_name);
    let mut file = File::create(&output_file)
        .await
        .with_context(|| format!("failed to create file '{}'", output_file.display()))?;
    file.write_all(output.as_bytes())
        .await
        .with_context(|| format!("failed to write file '{}'", output_file.display()))?;
    Ok(())
}
//based on https://stackoverflow.com/questions/26958489/how-to-copy-a-folder-recursively-in-rust
pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    let mut stack = Vec::new();
    stack.push(from.to_path_buf());

    let output_root = to.to_path_buf();
    let input_root = from.components().count();

    while let Some(working_path) = stack.pop() {
        // Generate a relative path
        let src: PathBuf = working_path.components().skip(input_root).collect();

        // Create a destination if missing
        let dest = if src.components().count() == 0 {
            output_root.clone()
        } else {
            output_root.join(&src)
        };
        if fs::metadata(&dest).is_err() {
            fs::create_dir_all(&dest)?;
        }

        for entry in fs::read_dir(working_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                match path.file_name() {
                    Some(filename) => {
                        let dest_path = dest.join(filename);
                        fs::copy(&path, &dest_path)?;
                    }
                    None => return Err(anyhow!("could not copy {:?}", path)),
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashSet;
    use std::fs::create_dir;
    use std::fs::File;
    use std::hash::Hash;
    use std::io::Write;
    use tempdir::TempDir;

    fn create_random_template_files(temp_dir: &Path, num_files: usize) -> Vec<PathBuf> {
        let mut file_names = Vec::new();

        for i in 0..num_files {
            let file_path = temp_dir.join(format!("file{}.template", i));
            let mut file = File::create(&file_path).unwrap();
            write!(
                file,
                "This is file {}.",
                file_path.file_name().unwrap().to_str().unwrap()
            )
            .unwrap();
            file_names.push(file_path);
        }

        file_names
    }

    fn iters_equal_any_order<T: Eq + Hash>(
        i1: impl Iterator<Item = T>,
        i2: impl Iterator<Item = T>,
    ) -> bool {
        let set: HashSet<T> = i2.collect();
        let vec: Vec<T> = i1.collect();
        vec.len() == set.len() && vec.into_iter().all(|x| set.contains(&x))
    }

    #[test]
    fn test_creates_reader_with_all_files() {
        let tempdir = TempDir::new("templates").unwrap();
        let files = create_random_template_files(tempdir.path(), 10);
        let reader = LazyFolderReader::new(tempdir.path(), "template").unwrap();
        assert!(iters_equal_any_order(
            files.into_iter(),
            reader.files.into_iter()
        ));
    }

    #[test]
    fn test_reader_into_iter() {
        let tempdir = TempDir::new("templates").unwrap();
        create_random_template_files(tempdir.path(), 5);
        let reader = LazyFolderReader::new(tempdir.path(), "template").unwrap();

        assert_eq!(5, reader.files.len());
        let mut checks = 0;
        for file in &reader {
            let file = file.unwrap();
            assert_eq!(format!("This is file {}.", file.name), file.contents);
            checks += 1;
        }
        assert_eq!(5, checks);
    }

    #[test]
    fn test_reader_iter() {
        let tempdir = TempDir::new("templates").unwrap();
        create_random_template_files(tempdir.path(), 5);
        let reader = LazyFolderReader::new(tempdir.path(), "template").unwrap();

        assert_eq!(5, reader.files.len());
        let mut checks = 0;
        for file in reader {
            let file = file.unwrap();
            assert_eq!(format!("This is file {}.", file.name), file.contents);
            checks += 1;
        }
        assert_eq!(5, checks);
    }

    #[test]
    fn test_reader_sub_dirs_iter() {
        let tempdir = TempDir::new("templates").unwrap();
        create_random_template_files(tempdir.path(), 5);
        let subdir = tempdir.path().join("subdir");
        create_dir(&subdir).unwrap();
        create_random_template_files(&subdir, 5);
        let reader = LazyFolderReader::new(tempdir.path(), "template").unwrap();

        // it should contain the 5 files on the main folder and the 5 in the subfolder
        assert_eq!(10, reader.files.len());
        let mut checks = 0;
        for file in reader {
            let file = file.unwrap();
            assert_eq!(format!("This is file {}.", file.name), file.contents);
            checks += 1;
        }
        assert_eq!(10, checks);
    }

    #[test]
    fn test_reader_excludes_wrong_extension() {
        let tempdir = TempDir::new("templates").unwrap();
        create_random_template_files(tempdir.path(), 3);
        for i in 0..2 {
            File::create(tempdir.path().join(format!("file{}.html", i))).unwrap();
        }
        let reader = LazyFolderReader::new(tempdir.path(), "template").unwrap();
        assert_eq!(3, reader.files.len());
    }

    #[test]
    fn test_reader_empty_dir() {
        let tempdir = TempDir::new("templates").unwrap();
        let reader = LazyFolderReader::new(tempdir.path(), "template").unwrap();
        assert!(reader.files.is_empty());
    }
}
