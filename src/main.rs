#[macro_use] mod macros;
mod cli;
mod pdf;

use pdf::reader::{load_pdf, load_input_as_pdf, get_pdf_size_in_kilobytes, get_compression_ratio_in_percent};
use pdf::writer::{compress_and_save_pdf, save_pdf};
use pdf::images::compress_images;
use pdf::merger::merge;
use pdf::builder::image_to_pdf;

use cli::args::{Cli, Commands, resolve_press_path_output, resolve_merge_path_output, resolve_convert_path_output};
use clap::Parser;

use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

fn expand_globs(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths.drain(..) {
        if path.exists() {
            result.push(path);
        } else {
            let s = match path.to_str() {
                Some(s) => s,
                None => { result.push(path); continue; }
            };
            match glob::glob(s) {
                Ok(entries) => {
                    let mut found = false;
                    for entry in entries {
                        match entry {
                            Ok(p) => { result.push(p); found = true; }
                            Err(e) => eprintln!("Warning: error matching '{}': {}", s, e),
                        }
                    }
                    if !found { result.push(path); }
                }
                Err(e) => {
                    eprintln!("Warning: invalid glob pattern '{}': {}", s, e);
                    result.push(path);
                }
            }
        }
    }
    result.sort_by(|a, b| {
        let a = a.to_str().unwrap_or("");
        let b = b.to_str().unwrap_or("");
        natord::compare(a, b)
    });
    result
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Press { input, output, quality, verbose } => {
            let input = expand_globs(input);
            let bar = ProgressBar::new(input.len() as u64);
            bar.set_style(ProgressStyle::default_bar()
                .template("{bar:40.cyan/blue} {pos}/{len} {eta}")
                .unwrap()
            );

            // Fail if multiple files + output are given & output is not a dir
            if input.len() > 1
                && let Some(ref path) = output
                && !path.is_dir() && !path.to_str().unwrap().ends_with('/') {
                eprintln!("Error: -o must be a directory when compressing multiple documents");
                std::process::exit(1);
            }

            // Create output dir if needed (once)
            if let Some(ref path) = output {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
                // If -o is a directory itself (ends with /)
                if path.to_str().unwrap().ends_with('/') {
                    std::fs::create_dir_all(path)?;
                }
            }
    
            for file_path in &input {
                // Loading the document
                let mut doc = match load_pdf(file_path.to_str().unwrap()) {
                    Ok(doc) => doc,
                    Err(e) => {
                        eprintln!("Skipping {}: {}", file_path.display(), e);
                        continue;
                    }
                };

                compress_images(&mut doc, quality, verbose);

                // Compressing the document
                let output = resolve_press_path_output(file_path, &output);
                compress_and_save_pdf(&mut doc, output.to_str().unwrap(), verbose)?;

                // Compression summary
                if verbose {
                    let original_size = get_pdf_size_in_kilobytes(file_path.to_str().unwrap()).unwrap();
                    let compressed_size = get_pdf_size_in_kilobytes(output.to_str().unwrap()).unwrap();
                    let compression_ratio = get_compression_ratio_in_percent(original_size, compressed_size);
                    bar.println(format!("{}kB → {}kB ({:.2}% compression)", original_size, compressed_size, compression_ratio));
                }

                bar.inc(1);
            }

            bar.finish_with_message("Done");
        }

        Commands::Merge { input, output, compress, optimize } => {
            let input = expand_globs(input);
            let mut documents = Vec::new();
            for path in &input {
                match load_input_as_pdf(path, false) {
                    Ok(d) => documents.push(d),
                    Err(e) => eprintln!("Skipping {}: {}", path.display(), e),
                }
            }

            // If compress => compress all inputs first then merge. If not, just merge.
            if compress {
                for doc in &mut documents {
                    compress_images(doc, 50, false);
                }
            }

            let output = resolve_merge_path_output(&output, compress);

            let mut merged = merge(documents)?;

            if optimize {
                pdf::optimizer::optimize(&mut merged);
            }

            save_pdf(&mut merged, output.to_str().unwrap())?;
        }

        Commands::Convert { input, output, merge: do_merge, compress, quality, verbose } => {
            let input = expand_globs(input);
            if do_merge {
                let mut docs = Vec::new();
                for path in &input {
                    match image_to_pdf(path, verbose) {
                        Ok(d) => docs.push(d),
                        Err(e) => eprintln!("Skipping {}: {}", path.display(), e),
                    }
                }

                let mut merged = merge(docs)?;
                if compress { compress_images(&mut merged, quality, verbose); }

                let out = resolve_merge_path_output(&output, compress);
                if compress {
                    compress_and_save_pdf(&mut merged, out.to_str().unwrap(), verbose)?;
                } else {
                    save_pdf(&mut merged, out.to_str().unwrap())?;
                }
            } else {
                // Same multi-file/-o guard as Press: -o must be a dir when >1 input.
                if input.len() > 1
                    && let Some(ref path) = output
                    && !path.is_dir() && !path.to_str().unwrap().ends_with('/') {
                    eprintln!("Error: -o must be a directory when converting multiple images");
                    std::process::exit(1);
                }

                for path in &input {
                    let mut doc = match image_to_pdf(path, verbose) {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("Skipping {}: {}", path.display(), e);
                            continue;
                        }
                    };

                    if compress { compress_images(&mut doc, quality, verbose); }

                    let out = resolve_convert_path_output(path, &output);
                    if compress {
                        compress_and_save_pdf(&mut doc, out.to_str().unwrap(), verbose)?;
                    } else {
                        save_pdf(&mut doc, out.to_str().unwrap())?;
                    }
                }
            }
        }
    }


    

    Ok(())
}
