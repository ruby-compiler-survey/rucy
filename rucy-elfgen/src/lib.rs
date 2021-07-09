use rucy_libelf_sys::*;
use std::ffi::c_void;
use std::fs::File;
use std::os::unix::io::IntoRawFd;

use errno::errno;

//use std::mem::MaybeUninit;
use std::path::Path;

mod models {
    #[derive(Debug, Clone)]
    pub struct Elf {
        pub ehdr: ElfHeader,
        pub phdr: Option<ProgHeader>,
        pub scns: Vec<Section>,
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct ElfHeader {
        pub r#type: u16,
        pub machine: u16,
        pub shstridx: u16,
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct ProgHeader {
        // TBA
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct Section {
        pub r#type: SectionType,
        pub header: SectionHeader,
        pub data: SectionHeaderData,
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct SectionHeader {
        pub name: String,
        pub name_idx: u32,
        pub r#type: u32,
        pub flags: u64,
        pub align: u64,
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub enum SectionHeaderData {
        Unset,
        Data(Vec<u8>),
        SymTab(Vec<Symbol>),
    }

    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct Symbol {
        pub name: String,
        pub name_idx: u32,
        pub shndx: u16,
    }

    #[derive(Debug, Copy, Clone)]
    #[non_exhaustive]
    #[allow(dead_code)]
    pub enum SectionType {
        Null,
        StrTab,
        Prog,
        License,
        SymTab,
    }
}

const DUMMY_BPF_PROG: &[u8] = b"\xb7\x00\x00\x00\x01\x00\x00\x00\x95\x00\x00\x00\x00\x00\x00\x00";

pub fn generate(path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut source = models::Elf {
        ehdr: models::ElfHeader {
            r#type: ET_REL as u16,
            machine: EM_BPF as u16,
            shstridx: 1,
        },
        phdr: None,
        scns: vec![
            models::Section {
                r#type: models::SectionType::StrTab,
                header: models::SectionHeader {
                    name: ".strtab".to_string(),
                    name_idx: 0,
                    r#type: SHT_STRTAB,
                    flags: 0,
                    align: 1,
                },
                data: models::SectionHeaderData::Unset,
            },
            models::Section {
                r#type: models::SectionType::License,
                header: models::SectionHeader {
                    name: "license".to_string(),
                    name_idx: 0,
                    r#type: SHT_PROGBITS,
                    flags: (SHF_ALLOC | SHF_WRITE) as u64,
                    align: 1,
                },
                data: models::SectionHeaderData::Data(b"GPL\0".to_vec()),
            },
            models::Section {
                r#type: models::SectionType::License,
                header: models::SectionHeader {
                    name: "license".to_string(),
                    name_idx: 0,
                    r#type: SHT_PROGBITS,
                    flags: (SHF_ALLOC | SHF_WRITE) as u64,
                    align: 1,
                },
                data: models::SectionHeaderData::Data(b"GPL\0".to_vec()),
            },
            models::Section {
                r#type: models::SectionType::Prog,
                header: models::SectionHeader {
                    name: "cgroup/dev".to_string(),
                    name_idx: 0,
                    r#type: SHT_PROGBITS,
                    flags: (SHF_ALLOC | SHF_EXECINSTR) as u64,
                    align: 8,
                },
                data: models::SectionHeaderData::Data(DUMMY_BPF_PROG.to_vec()),
            },
        ],
    };

    unsafe {
        elf_version(EV_CURRENT);
        let file = File::create(path)?;
        let elf = elf_begin(
            file.into_raw_fd(),
            Elf_Cmd_ELF_C_WRITE,
            std::ptr::null_mut::<Elf>(),
        );
        if elf.is_null() {
            return Err(Box::new(errno()));
        }

        let mut ehdr = elf64_newehdr(elf);
        (*ehdr).e_type = source.ehdr.r#type;
        (*ehdr).e_machine = source.ehdr.machine;
        (*ehdr).e_shstrndx = source.ehdr.shstridx;

        if gelf_update_ehdr(elf, ehdr) == 0 {
            return Err(Box::new(errno()));
        }

        // skip phdr

        let mut strtab = String::new();
        strtab.push('\0');

        for scn in source.scns.iter_mut() {
            scn.header.name_idx = (strtab.len()) as u32;
            strtab.push_str(&scn.header.name);
            strtab.push('\0');
        }

        for scn in source.scns.iter_mut() {
            let scn_ = elf_newscn(elf);
            let sh = elf64_getshdr(scn_);
            let data_ = elf_newdata(scn_);

            use models::SectionType;
            let ty = scn.r#type;
            match ty {
                SectionType::StrTab => {
                    let mut buf = vec![0u8; strtab.len()].into_boxed_slice();
                    buf.copy_from_slice(strtab.as_bytes());
                    let len = buf.len() as u64;
                    (*data_).d_buf = Box::into_raw(buf) as *mut c_void;
                    (*data_).d_size = len;
                    (*data_).d_align = scn.header.align;

                    (*sh).sh_size = len;
                    (*sh).sh_entsize = 0;
                }
                SectionType::License | SectionType::Prog => {
                    if let models::SectionHeaderData::Data(data) = &scn.data {
                        let mut buf = vec![0u8; data.len()].into_boxed_slice();
                        buf.copy_from_slice(data);
                        let len = buf.len() as u64;
                        (*data_).d_buf = Box::into_raw(buf) as *mut c_void;
                        (*data_).d_size = len;
                        (*data_).d_align = scn.header.align;

                        (*sh).sh_size = len;
                        (*sh).sh_entsize = 0;
                    } else {
                        panic!("invalid data: {:?}", scn.data);
                    }
                }
                _ => {
                    panic!("unsupported: {:?}", ty);
                }
            }

            (*sh).sh_type = scn.header.r#type;
            (*sh).sh_addralign = scn.header.align;
            (*sh).sh_flags = scn.header.flags;
            (*sh).sh_name = scn.header.name_idx;

            if gelf_update_shdr(scn_, sh) == 0 {
                return Err(Box::new(errno()));
            }
        }

        if elf_update(elf, Elf_Cmd_ELF_C_WRITE) == -1 {
            return Err(Box::new(errno()));
        }

        if elf_end(elf) == -1 {
            return Err(Box::new(errno()));
        }
    }
    println!("¡Hola!");
    Ok(())
}