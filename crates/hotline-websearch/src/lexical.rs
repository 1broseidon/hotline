//! Whether a word could be a word. The "no result mentions" warning is for
//! words that are plainly not ordinary ones (a made-up token, an id), and an
//! ordinary word in any language must never trip it. A small table of the
//! letter pairs real words use, drawn from a sample of six languages, does it.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Everyday words in English, French, Spanish, German, Italian and Portuguese.
/// Only their letter pairs are kept.
const SAMPLE: &str = "
the people because through between another information government country example question
problem different important development company national international business system program
children world during without before around however something nothing everything
strength rhythm length months weather height official altitude mountain protocol semantic
technology software hardware network internet computer application language search results
history science university education research market general public private service music
pourquoi comment parce hauteur officielle montagne altitude sommet gouvernement entreprise
information développement système programme enfants monde pendant avant aussi toujours
rien quelque chose exemple question problème différent important français européen général
service musique histoire science université recherche résultat météo température journée
porque cómo también donde altura oficial montaña gobierno empresa información desarrollo
sistema programa niños mundo durante antes siempre nada algo ejemplo pregunta problema
diferente importante español europeo general servicio música historia ciencia universidad
warum wie weil höhe offiziell berg gebirge regierung unternehmen entwicklung system
programm kinder welt während vor immer nichts etwas beispiel frage problem unterschiedlich
wichtig deutsch europäisch allgemein dienst musik geschichte wissenschaft universität
forschung ergebnis wetter temperatur zeitgeist schadenfreude straße schlüssel
perché come altezza ufficiale montagna governo azienda informazione sviluppo sistema
programma bambini mondo durante prima sempre niente qualcosa esempio domanda problema
differente importante italiano europeo generale servizio musica storia scienza università
ricerca risultato tempo temperatura giornata
porquê altura oficial montanha governo empresa informação desenvolvimento sistema
programa crianças mundo durante antes sempre nada algo exemplo pergunta problema
diferente importante português europeu geral serviço música história ciência universidade
pesquisa resultado tempo temperatura jornada
quick brown jumping lazy kubernetes docker server client database query index cache thread
async await tokio runtime compile compiler error warning kernel linux window browser
python javascript typescript golang rust cargo crate package module library framework
everest himalaya nepal tibet china india france germany spain italy portugal london paris
berlin madrid rome lisbon wikipedia google apple microsoft amazon facebook twitter youtube
items systems programs forms rhythms strengths lengths
quarter question quote squad square equal queen quiet quit
";

static PAIRS: LazyLock<HashSet<(char, char)>> = LazyLock::new(|| {
    let mut pairs = HashSet::new();
    for word in SAMPLE.split_whitespace() {
        let letters: Vec<char> = word.to_lowercase().chars().collect();
        for pair in letters.windows(2) {
            pairs.insert((pair[0], pair[1]));
        }
    }
    pairs
});

fn is_vowel(c: char) -> bool {
    "aeiouyàâäáãåæèéêëìíîïòóôöõøùúûüœ".contains(c)
}

/// Whether `word` reads as an ordinary word: made of letter pairs real words
/// use, with vowels in it, and not a long run of consonants. Words with digits
/// or too short to judge are not this function's business and count as words.
pub fn looks_lexical(word: &str) -> bool {
    let letters: Vec<char> = word.to_lowercase().chars().collect();
    if letters.len() < 4 || !letters.iter().all(|c| c.is_alphabetic()) {
        return true;
    }
    let vowels = letters.iter().filter(|c| is_vowel(**c)).count();
    if vowels == 0 {
        return false;
    }
    let mut run = 0;
    let mut longest = 0;
    for c in &letters {
        if is_vowel(*c) {
            run = 0;
        } else {
            run += 1;
            longest = longest.max(run);
        }
    }
    let known = letters
        .windows(2)
        .filter(|pair| PAIRS.contains(&(pair[0], pair[1])))
        .count();
    let score = known as f64 / (letters.len() - 1) as f64;
    // Unfamiliar pairs make a word suspect; a long consonant run makes it more so.
    score >= 0.75 && !(longest >= 4 && score < 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_words_in_several_languages_are_words() {
        for word in [
            "officielle",
            "hauteur",
            "altitude",
            "protocol",
            "semantics",
            "kubernetes",
            "everest",
            "météo",
            "paris",
            "wikipedia",
            "quantum",
            "photosynthesis",
            "mitochondria",
            "worcestershire",
            "strengths",
            "rhythms",
            "schadenfreude",
            "weltanschauung",
            "straßenbahn",
            "cuaderno",
            "fotografia",
            "gnocchi",
            "perspective",
            "tokio",
            "anthropic",
            "typescript",
            "sanctions",
            "iphone",
            "bitcoin",
            "ethereum",
            "nginx",
            "postgres",
            "qatar",
            "zeitgeist",
            "chrysanthemum",
            "bureaucracy",
        ] {
            assert!(looks_lexical(word), "{word}");
        }
    }

    #[test]
    fn made_up_strings_are_not() {
        for word in [
            "xqzflarnib",
            "qzxvbn",
            "bcdfgh",
            "zxqwvk",
            "hjkqwrt",
            "flrnbq",
        ] {
            assert!(!looks_lexical(word), "{word}");
        }
    }

    #[test]
    fn digits_and_short_words_are_not_judged() {
        assert!(looks_lexical("e0502"));
        assert!(looks_lexical("xq"));
    }
}
