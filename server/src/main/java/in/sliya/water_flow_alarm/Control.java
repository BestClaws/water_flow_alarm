package in.sliya.water_flow_alarm;

import org.springframework.stereotype.Controller;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.ResponseBody;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class Control {

    @GetMapping("/update/{val}")
    public void updateVal(@PathVariable(name = "val") int val) {
        System.out.println(val);
    }
}
