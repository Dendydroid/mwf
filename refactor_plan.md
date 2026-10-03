### Description
There's currently a lot of problems in this project during dialogues so I suggest we rewrite the machines we have and the process.
Known issues are:
- Bad matching of intents, sometimes can match get_calendar, when its absolutely not
- Field values are confirmed by 'here' utterance and it might be that some other variants work
- 'Is there anything else you want help with?' after value was provided and should've been a confirmation of value instead or if it was a confirmation then next value, also anything else is ambigious
- When confirming a value badly formulated sentences (was in Russian) wasnt clear that it asked for confirmation

All of these reasons and many other findings lead to this refactor document.

### Suggested new approach

Generally what I have in mind is that we need to restrict the number of intents that can be matched according to current context as much as we can to avoid bad intent matches as much as possible.
You can imagine this as a MENU in the game. A main menu has categories and actions, its not possible to click change FOV in main menu, its only possible in video settings and so on. So this is restricted in a sense.
Also menus are connected, there's certain hierarchy, so that's what im going to try to replicate here.
Also I think more focused smaller in terms of their responsibility 'machines' which are also scoped to the context (menu) would help resolve most of the problems with flows.


### New Machines (in flow order)

#### InfoExtractor
This machine will be the first one in the flow, it will extract values.
It will only be part of the flow if no form is currently active.
It has a struct with values that can be filled out, it is not dynamic, its predefined with predefined types.
Each utterance when there's no form active the values get either set or updated.
One value from utterance can only set/update 1 property. So if utterance contains for example 1999-06-13, this can only fill out 1 property the one that fits the most.
Example pseudocode:
HintMap {
caller_full_name: Some("Joshua Blake"),
patient_full_name: None,
spoken_appointment_datetime: Some("Next saturday"),
exact_appointment_datetime: Some("2026-10-12 14:00"),
exact_birth_date: None,
}

Stored in session.

Here this map doesnt include a description per property, but they have to be defined as a static text for llm's reasoning so it can understand which one to fill out.
If something was said twice we override the value.

Once any form is started, this struct has to be convertable to Form, maybe with From<> trait if that's feasable.
Or a callback can be entered in the Form::new arguments which takes in the HintMap and prefills out some values if applicable.
All the form fields are still UNCONFIRMED at that point and they will be confirmed later.

When Form finishes successfully, HintMap gets form values back to update the HintMap. So basically they can be transformed back and forth into one another, maybe another callback in Form::new, or since Form uses a builder pattern maybe to_hintmap and from_hintmap can be added and those callbacks either Some or None and then used.
When values are converted back from form to hintmap its updated in session and can be used again for other processes and context.

### FormInfoExtractor
This machine will be the first one in the flow, it will extract values.
It will only be part of the flow if FORM IS CURRENTLY ACTIVE.
Each said values from utterance can set/update only 1 property.
The values once set or updated are always unconfirmed.
Extractor receives conversation history plus current step which needs to be filled out in the context.

### MainIntentMatcher
This machine will select one intent out of all allowed intents as it works now, but now it will have not all intents.
The intents which are only expected during Form filling will be handled by FormIntentMatcher.
Once intent start form is matched, this machine will be replaced by FormIntentMatcher, the only thing this machine will be able to do is starting a form and passing info about first field to response formulator for first question.
Additional intent will be added here: greeting, so unsupported doesnt match when person greets. Greeting also will result in listing possibilities and a friendly greeting and presenting AI assistant to caller.
Additional intent will be added here: get_information[this_call]. its going to use information from HintMap to answer callers question.
Once Form is finished, MainIntentMatcher takes over the flow again back from FormIntentMatcher.
A new additional intent and get_information is added to both here and FormIntentMatcher, its who_are_you which ai assistant will answer explaining.

### FormIntentMatcher
This machine will select one intent out of all allowed intents which are relevant for active form.
This machine can also handle allowed get_information during filling a form.
Correction of field value is performed by it.
Setting of field value is performed by it.
End call will result in first asking a confirmation, because form is active.
Call human is possible.
Repeat last sentence/question is possible.
Providing value now can contain multiple values of the form.
If confirming values succeeded, next queued step is selected.
If confirming values succeeded when final form confirmation was asked, the form is now finished and that context is added for response formulator.
If confirming values failed, these values become empty and queued again so it can be reasked.
If confirming values failed when final form confirmation was asked, the form is dropped and started from the beginning and that is added to context for response formulator.

### ResponseFormulator
This machine will formulate a coherent answer to all utterances. It will use context to do that.
If form is active, its state is provided in the query as well as that its active.
If main intent matcher says form has started and provides the first field we ask for it.
The form intent matcher provides the current form state.
Then a sentence must be formulated.

If form only started then we just ask the field value.

If value/values was/were provided then we read it/them back and ask for confirmation.

If confirmation success was provided then we thank caller and ask next value if form is not finished, or state that the form has finished with full summary.

If confirmation failed then we apologise and ask current pending value (Regardless if that was failed confirmation of field/fields or a final form confirmation because form should be dropped at this point and next pending field is first one)

If get information was asked we provide the information and if there's
- a pending field value we ask for it again
- a pending confirmation for value/values we ask for confirmation again

