/**
 * The names the die in New teammate rolls from: short, friendly, easy to
 * say out loud on a call, from many places, and none of them an AI
 * product or a model family, so a teammate never sounds like a vendor.
 */
export const NAMES: readonly string[] = [
	"Ada", "Scout", "Penny", "Milo", "Juno", "Otto", "Iris", "Felix", "Rosa", "Theo", "Luna", "Gus", "Hazel", "Remy",
	"Ivy", "Max", "Wren", "Leo", "Mabel", "Finn", "Clara", "Ozzy", "Bea", "Sage", "Hugo", "Pip", "Nell", "Kit", "Arlo",
	"Abe", "Addie", "Aggie", "Alba", "Alfie", "Alma", "Amos", "Anya", "Archie", "Arden", "Asa", "Astrid", "Audie",
	"Avery", "Basil", "Benji", "Bess", "Birdie", "Blair", "Bodhi", "Bram", "Brie", "Bruno", "Cal", "Callie", "Cass",
	"Cece", "Cleo", "Cody", "Cora", "Cosmo", "Dahlia", "Dash", "Daisy", "Della", "Dex", "Dot", "Drew", "Edie", "Effie",
	"Elio", "Eliza", "Ellis", "Elsie", "Emery", "Enzo", "Esme", "Etta", "Ezra", "Fern", "Fig", "Flo", "Flynn", "Frankie",
	"Fritz", "Gemma", "Georgie", "Gigi", "Gil", "Ginny", "Goldie", "Greta", "Gwen", "Hal", "Hank", "Harper", "Hattie",
	"Hollis", "Honey", "Huck", "Ida", "Ike", "Indie", "Ines", "Ingrid", "Isla", "Jack", "Jade", "Jas", "Jett", "Jojo",
	"Jude", "June", "Juniper", "Kai", "Kaya", "Keats", "Kiki", "Kip", "Koa", "Lark", "Laszlo", "Lena", "Lev", "Lila",
	"Lola", "Lottie", "Lou", "Lucy", "Lulu", "Lyle", "Mack", "Maeve", "Mae", "Marlo", "Mars", "Matty", "Maya", "Merle",
	"Mila", "Mimi", "Minnie", "Mo", "Monty", "Moss", "Murphy", "Nash", "Nia", "Nico", "Nina", "Noor", "Norah", "Nora",
	"Ollie", "Opal", "Oscar", "Pablo", "Paz", "Pearl", "Percy", "Petra", "Phoebe", "Pia", "Piper", "Polly", "Poppy",
	"Quincy", "Quinn", "Rafa", "Ramona", "Ray", "Reggie", "Rhea", "Rio", "Rocco", "Romy", "Ronan", "Rory", "Roux", "Ruby",
	"Rufus", "Rumi", "Ruth", "Sadie", "Sal", "Sami", "Sasha", "Sid", "Sienna", "Silas", "Sky", "Sol", "Stella", "Stevie",
	"Sunny", "Suki", "Tad", "Tali", "Tess", "Tilly", "Tobi", "Toby", "Tuck", "Uma", "Una", "Val", "Vera", "Vida", "Vince",
	"Viola", "Vito", "Walt", "Wally", "Willa", "Willow", "Winnie", "Wyatt", "Xavi", "Yara", "Yuki", "Yusuf", "Zara",
	"Zeke", "Ziggy", "Zoe", "Zola", "Ari", "Bix", "Bodie", "Coco", "Dino", "Elm", "Fox", "Gale", "Gray", "Haru", "Jem",
	"Kenji", "Kofi", "Lumi", "Mika", "Niko", "Obi", "Odie", "Ori", "Pax", "Remi", "Rex", "Soren", "Teo", "Timo", "Vesper",
	"Wiley", "Yoshi", "Zane", "Ansel", "Beau", "Bonnie", "Cedar", "Dory", "Edda", "Faye", "Gio", "Jory", "Kaz", "Lennox",
	"Marigold", "Ned", "Orla", "Pike", "Rosie", "Saoirse", "Thea", "Tova", "Ulla", "Vivi", "Wilder", "Zinnia", "Ash",
	"Bryn", "Calla", "Dara", "Ferris", "Gwyn", "Jonah", "Lior", "Mabry", "Nyla", "Oona", "Priya", "Rania", "Selma",
	"Tomas", "Uri", "Vale", "Wynn", "Yael", "Zev", "Amara", "Benny", "Corin",
];

/** A name from the list at random, never the one already showing. */
export function suggestName(current: string, random: () => number = Math.random): string {
	const others = NAMES.filter((one) => one !== current.trim());
	return others[Math.floor(random() * others.length)] ?? NAMES[0]!;
}
