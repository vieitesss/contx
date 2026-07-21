#[allow(unused)]

#[derive(Debug)]
pub struct TmuxSession {
    pub name: String,
}

pub fn parse(output: &str) -> Vec<TmuxSession> {
    let mut list: Vec<TmuxSession> = Vec::new();
    for line in output.lines() {
        let name = line.split(":").collect::<Vec<&str>>()[0];
        let s = TmuxSession {
            name: name.to_string(),
        };
        list.push(s);
    }
    list
}
