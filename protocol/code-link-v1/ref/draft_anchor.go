package codelinkv1ref

// This file is the EXTERNAL ANCHOR for the CPace-Ristretto255 test vectors. The
// constants below are transcribed BY HAND from draft-irtf-cfrg-cpace-15
// Appendix B.3 (and B.3.10/B.3.11) — they are NOT generated from this package.
// VerifyDraftVectors recomputes each value with the implementation under test
// and returns an error if any differs from these literals, so a broken
// implementation cannot "pass" by regenerating its own goldens (blocker 2). The
// fixture generator and the test both call VerifyDraftVectors; either fails
// loudly on a mismatch.
//
// Source: https://www.ietf.org/archive/id/draft-irtf-cfrg-cpace-15.txt §B.3.

import (
	"bytes"
	"encoding/hex"
	"fmt"
)

// DraftB3 holds the literal draft-15 §B.3 values (hex, exactly as printed).
var DraftB3 = struct {
	PRSHex, CIHex, SidHex                 string
	YaScalarHex, YbScalarHex              string
	AdaHex, AdbHex                        string
	GeneratorStringHex, GHex              string
	YaHex, YbHex, KHex                    string
	TranscriptIRHex, TranscriptOCHex      string
	ISKIRHex, ISKOCHex                    string
	SidOutputLabel                        string
	SidOutputIRHex, SidOutputOCHex        string
	// §B.3.10 scalar_mult_vfy with valid inputs.
	VfyScalarHex, VfyXHex, VfyResultHex   string
	// §B.3.11 invalid inputs (both must map to identity => abort).
	InvalidY1Hex, InvalidIdentityHex      string
}{
	PRSHex:             "50617373776f7264",                                                 // b"Password"
	CIHex:              "6f630b425f726573706f6e6465720b415f696e69746961746f72",             // b"oc\x0bB_responder\x0bA_initiator"
	SidHex:             "7e4b4791d6a8ef019b936c79fb7f2c57",
	YaScalarHex:        "da3d23700a9e5699258aef94dc060dfda5ebb61f02a5ea77fad53f4ff0976d08",
	YbScalarHex:        "d2316b454718c35362d83d69df6320f38578ed5984651435e2949762d900b80d",
	AdaHex:             "414461", // b"ADa"
	AdbHex:             "414462", // b"ADb"
	GeneratorStringHex: "11435061636552697374726574746f3235350850617373776f726464" +
		"00000000000000000000000000000000000000000000000000000000" +
		"00000000000000000000000000000000000000000000000000000000" +
		"00000000000000000000000000000000000000000000000000000000" +
		"000000000000000000000000000000001a6f630b425f726573706f6e" +
		"6465720b415f696e69746961746f72107e4b4791d6a8ef019b936c79" +
		"fb7f2c57",
	GHex:            "a6fc82c3b8968fbb2e06fee81ca858586dea50d248f0c7ca6a18b0902a30b36b",
	YaHex:           "d40fb265a7abeaee7939d91a585fe59f7053f982c296ec413c624c669308f87a",
	YbHex:           "08bcf6e9777a9c313a3db6daa510f2d398403319c2341bd506a92e672eb7e307",
	KHex:            "e22b1ef7788f661478f3cddd4c600774fc0f41e6b711569190ff88fa0e607e09",
	TranscriptIRHex: "20d40fb265a7abeaee7939d91a585fe59f7053f982c296ec413c624c" +
		"669308f87a034144612008bcf6e9777a9c313a3db6daa510f2d39840" +
		"3319c2341bd506a92e672eb7e30703414462",
	TranscriptOCHex: "6f6320d40fb265a7abeaee7939d91a585fe59f7053f982c296ec413c" +
		"624c669308f87a034144612008bcf6e9777a9c313a3db6daa510f2d3" +
		"98403319c2341bd506a92e672eb7e30703414462",
	ISKIRHex: "4c5469a16b2364c4b944ebc1a79e51d1674ad47db26e8718154f59fa" +
		"ebfaa52d8346f30aa58377117eb20d527f2cbc5c76381f7fd372e89d" +
		"f8239f87f2e02ed1",
	ISKOCHex: "980dcc5a1c52ceea031e75f38ed266586616488c5c5780285fcbcf79" +
		"087c7bcdbd993502eee606b718ba31e840a000a7b7befe15ea427c5c" +
		"fe88344fa1237f35",
	SidOutputLabel: "CPaceSidOutput",
	SidOutputIRHex: "2a76d3bbc499dfdc4dcacc9ff042f4e1a54e3843258e100ccd7c60f0" +
		"a541f9d3ebf025e68a460dde218bd39f0711bc6fa11409c9d7b69d8c" +
		"cf6b32fc51ddb699",
	SidOutputOCHex: "ca4b50700c46203ccd10bc0e9f31095e508189cb59857537be561048" +
		"d34b9ed9a9697af11c998f484c3d783b0b531434caa6835d4c32344f" +
		"cd17160c9c348fc7",
	VfyScalarHex:      "7cd0e075fa7955ba52c02759a6c90dbbfc10e6d40aea8d283e407d88cf538a05",
	VfyXHex:           "2c3c6b8c4f3800e7aef6864025b4ed79bd599117e427c41bd47d93d654b4a51c",
	VfyResultHex:      "7c13645fe790a468f62c39beb7388e541d8405d1ade69d1778c5fe3e7f6b600e",
	InvalidY1Hex:      "2b3c6b8c4f3800e7aef6864025b4ed79bd599117e427c41bd47d93d654b4a51c",
	InvalidIdentityHex: "0000000000000000000000000000000000000000000000000000000000000000",
}

func ahex(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic("draft_anchor: bad literal hex: " + err.Error())
	}
	return b
}

// VerifyDraftVectors recomputes every draft §B.3 value from the implementation
// and asserts byte-equality against the hand-transcribed literals above. It is
// the non-tautological gate: the literals are the external source of truth.
func VerifyDraftVectors() error {
	d := DraftB3
	prs, ci, sid := ahex(d.PRSHex), ahex(d.CIHex), ahex(d.SidHex)

	check := func(name, gotHex, wantHex string) error {
		if gotHex != wantHex {
			return fmt.Errorf("draft %s:\n  got  %s\n  want %s", name, gotHex, wantHex)
		}
		return nil
	}

	// generator_string + generator g.
	gs := GeneratorString([]byte(DSI), prs, ci, sid, SHA512BlockBytes)
	if err := check("generator_string", hex.EncodeToString(gs), d.GeneratorStringHex); err != nil {
		return err
	}
	g, err := CalculateGenerator(prs, ci, sid)
	if err != nil {
		return err
	}
	if err := check("g", hex.EncodeToString(EncodeElement(g)), d.GHex); err != nil {
		return err
	}

	// Ya, Yb from fixed scalars.
	ya, err := ScalarFromCanonical(ahex(d.YaScalarHex))
	if err != nil {
		return err
	}
	yb, err := ScalarFromCanonical(ahex(d.YbScalarHex))
	if err != nil {
		return err
	}
	if err := check("Ya", hex.EncodeToString(ScalarMult(ya, g)), d.YaHex); err != nil {
		return err
	}
	if err := check("Yb", hex.EncodeToString(ScalarMult(yb, g)), d.YbHex); err != nil {
		return err
	}

	// K from both directions (via the validated ScalarMultVfy path).
	kA, err := ScalarMultVfy(ya, ahex(d.YbHex))
	if err != nil {
		return err
	}
	kB, err := ScalarMultVfy(yb, ahex(d.YaHex))
	if err != nil {
		return err
	}
	if err := check("K(ya,Yb)", hex.EncodeToString(kA.bytes()), d.KHex); err != nil {
		return err
	}
	if err := check("K(yb,Ya)", hex.EncodeToString(kB.bytes()), d.KHex); err != nil {
		return err
	}

	// Transcripts, ISK (both orderings), sid-output.
	ada, adb := ahex(d.AdaHex), ahex(d.AdbHex)
	Ya, Yb := ahex(d.YaHex), ahex(d.YbHex)
	trIR := TranscriptIR(Ya, ada, Yb, adb)
	trOC := TranscriptOC(Ya, ada, Yb, adb)
	if err := check("transcript_ir", hex.EncodeToString(trIR), d.TranscriptIRHex); err != nil {
		return err
	}
	if err := check("transcript_oc", hex.EncodeToString(trOC), d.TranscriptOCHex); err != nil {
		return err
	}
	if err := check("ISK_ir", hex.EncodeToString(ISK(sid, kA, trIR)), d.ISKIRHex); err != nil {
		return err
	}
	if err := check("ISK_oc", hex.EncodeToString(ISK(sid, kA, trOC)), d.ISKOCHex); err != nil {
		return err
	}
	if err := check("sid_output_ir", hex.EncodeToString(SidOutput(d.SidOutputLabel, trIR)), d.SidOutputIRHex); err != nil {
		return err
	}
	if err := check("sid_output_oc", hex.EncodeToString(SidOutput(d.SidOutputLabel, trOC)), d.SidOutputOCHex); err != nil {
		return err
	}

	// §B.3.10 scalar_mult_vfy with valid inputs.
	vs, err := ScalarFromCanonical(ahex(d.VfyScalarHex))
	if err != nil {
		return err
	}
	vfy, err := ScalarMultVfy(vs, ahex(d.VfyXHex))
	if err != nil {
		return fmt.Errorf("draft B.3.10 valid scalar_mult_vfy aborted: %w", err)
	}
	if err := check("scalar_mult_vfy", hex.EncodeToString(vfy.bytes()), d.VfyResultHex); err != nil {
		return err
	}

	// §B.3.11 invalid inputs: both MUST abort (map to identity / bad encoding).
	if _, err := ScalarMultVfy(vs, ahex(d.InvalidY1Hex)); err == nil {
		return fmt.Errorf("draft B.3.11 Y_i1 (invalid encoding) was accepted, must abort")
	}
	if _, err := ScalarMultVfy(vs, ahex(d.InvalidIdentityHex)); err == nil {
		return fmt.Errorf("draft B.3.11 Y_i2 (identity) was accepted, must abort")
	}

	// Sanity: the invalid Y_i1 really is one bit off the valid X (2b vs 2c),
	// i.e. a genuinely close-but-invalid encoding, not a typo in the anchor.
	if bytes.Equal(ahex(d.VfyXHex), ahex(d.InvalidY1Hex)) {
		return fmt.Errorf("draft anchor: valid X and invalid Y_i1 are identical")
	}
	return nil
}
